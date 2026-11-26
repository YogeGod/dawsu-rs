//! Weighted-DAG Plugin Delay Compensation (PDC) solver and pre-allocated lock-free delay lines.

/// Node in a directed acyclic audio routing graph used to compute per-edge compensation delays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdcNode {
    pub id: u32,
    pub own_latency: u32,
    pub outputs: Vec<u32>,
}

/// Compensation delay (in samples) inserted along a directed edge `from -> to`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PdcEdgeDelay {
    pub from: u32,
    pub to: u32,
    pub delay_samples: u32,
}

/// Complete PDC solution for a routing graph.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PdcPlan {
    pub topo_order: Vec<u32>,
    pub arrival_latency: Vec<(u32, u32)>,
    pub edge_delays: Vec<PdcEdgeDelay>,
    pub total_latency: u32,
}

impl PdcPlan {
    /// Returns the compensation delay along edge `from -> to`, or `0` if absent.
    pub fn delay_for_edge(&self, from: u32, to: u32) -> u32 {
        self.edge_delays
            .iter()
            .find(|edge| edge.from == from && edge.to == to)
            .map_or(0, |edge| edge.delay_samples)
    }

    /// Returns the cumulative arrival latency before node `id` begins processing.
    pub fn arrival_of(&self, id: u32) -> u32 {
        self.arrival_latency
            .iter()
            .find(|(node_id, _)| *node_id == id)
            .map_or(0, |(_, lat)| *lat)
    }
}

/// In-place, zero-allocation DAG longest-path PDC solver used by both `compute_pdc` and the RT
/// mixer graph when a plug-in reports `kLatencyChanged`.
///
/// Preconditions (debug-checked): `in_degree.len() == own_latency.len()`,
/// `topo.len() == own_latency.len()`, `arrival.len() == own_latency.len()`, and
/// `edge_delays.len() == edges.len()`.
pub(crate) fn solve_dag_delays_in_place(
    own_latency: &[u32],
    edges: &[(usize, usize)],
    edge_delays: &mut [u32],
    in_degree: &mut [usize],
    topo: &mut [usize],
    arrival: &mut [u32],
) -> Result<u32, &'static str> {
    let n = own_latency.len();
    if n == 0 {
        return Ok(0);
    }
    in_degree.fill(0);
    arrival.fill(0);
    for &(from, to) in edges {
        if from >= n || to >= n || from == to {
            return Err("Invalid or self-loop edge in PDC graph");
        }
        in_degree[to] += 1;
    }

    let mut head = 0usize;
    let mut tail = 0usize;
    for (idx, &deg) in in_degree.iter().enumerate() {
        if deg == 0 {
            topo[tail] = idx;
            tail += 1;
        }
    }

    while head < tail {
        let u = topo[head];
        head += 1;
        let out_lat = arrival[u].saturating_add(own_latency[u]);
        for &(from, to) in edges {
            if from == u {
                if out_lat > arrival[to] {
                    arrival[to] = out_lat;
                }
                in_degree[to] -= 1;
                if in_degree[to] == 0 {
                    topo[tail] = to;
                    tail += 1;
                }
            }
        }
    }

    if tail != n {
        return Err("Cycle detected in PDC routing graph");
    }

    for (edge_idx, &(from, to)) in edges.iter().enumerate() {
        let branch_finish = arrival[from].saturating_add(own_latency[from]);
        edge_delays[edge_idx] = arrival[to].saturating_sub(branch_finish);
    }

    let mut total_latency = 0u32;
    for idx in 0..n {
        total_latency = total_latency.max(arrival[idx].saturating_add(own_latency[idx]));
    }
    Ok(total_latency)
}

/// Computes topological order, arrival latencies, compensation delays per directed edge, and
/// overall graph latency for a weighted DAG. Rejects cycles and unknown/duplicate node IDs.
pub fn compute_pdc(nodes: &[PdcNode]) -> Result<PdcPlan, String> {
    let mut own_latency = Vec::with_capacity(nodes.len());
    for (idx, node) in nodes.iter().enumerate() {
        if nodes[..idx].iter().any(|prev| prev.id == node.id) {
            return Err(format!("Duplicate PDC node id {}", node.id));
        }
        own_latency.push(node.own_latency);
    }

    let index_of = |id: u32| -> Option<usize> { nodes.iter().position(|node| node.id == id) };
    let mut edges = Vec::new();
    for (from_idx, node) in nodes.iter().enumerate() {
        for &target_id in &node.outputs {
            let to_idx = index_of(target_id)
                .ok_or_else(|| format!("PDC edge {} -> {} references missing node", node.id, target_id))?;
            edges.push((from_idx, to_idx));
        }
    }

    let mut edge_delay_vals = vec![0u32; edges.len()];
    let mut in_degree = vec![0usize; nodes.len()];
    let mut topo = vec![0usize; nodes.len()];
    let mut arrival = vec![0u32; nodes.len()];
    let total_latency = solve_dag_delays_in_place(
        &own_latency,
        &edges,
        &mut edge_delay_vals,
        &mut in_degree,
        &mut topo,
        &mut arrival,
    )
    .map_err(str::to_string)?;

    let topo_order = topo.iter().map(|&idx| nodes[idx].id).collect();
    let arrival_latency = nodes
        .iter()
        .enumerate()
        .map(|(idx, node)| (node.id, arrival[idx]))
        .collect();
    let edge_delays = edges
        .iter()
        .enumerate()
        .map(|(edge_idx, &(from, to))| PdcEdgeDelay {
            from: nodes[from].id,
            to: nodes[to].id,
            delay_samples: edge_delay_vals[edge_idx],
        })
        .collect();

    Ok(PdcPlan {
        topo_order,
        arrival_latency,
        edge_delays,
        total_latency,
    })
}

/// Pre-allocated, lock-free stereo delay line with 10 ms click-free crossfade on delay changes.
///
/// Continuously records incoming frames whenever `max_delay > 0` so runtime latency changes can
/// jump up to `max_delay` samples without heap allocations or stale history.
pub struct DelayLine {
    left: Vec<f32>,
    right: Vec<f32>,
    write_pos: usize,
    max_delay: usize,
    current_delay: usize,
    prev_delay: usize,
    crossfade_pos: usize,
    crossfade_len: usize,
}

impl DelayLine {
    /// Creates a delay line with capacity up to `max_delay_samples` frames and a 10 ms crossfade.
    pub fn new(max_delay_samples: usize, sample_rate: u32) -> Self {
        let len = max_delay_samples.saturating_add(1);
        let crossfade_len = (sample_rate.max(1) as usize / 100).max(1);
        Self {
            left: vec![0.0; len],
            right: vec![0.0; len],
            write_pos: 0,
            max_delay: max_delay_samples,
            current_delay: 0,
            prev_delay: 0,
            crossfade_pos: crossfade_len,
            crossfade_len,
        }
    }

    pub fn max_delay_samples(&self) -> usize {
        self.max_delay
    }

    pub fn current_delay_samples(&self) -> usize {
        self.current_delay
    }

    /// Sets the delay immediately without crossfading (intended for initial off-thread setup).
    pub fn set_delay_immediate(&mut self, samples: usize) {
        let clamped = samples.min(self.max_delay);
        self.current_delay = clamped;
        self.prev_delay = clamped;
        self.crossfade_pos = self.crossfade_len;
    }

    /// Smoothly crossfades to `target` samples over ~10 ms using zero heap allocations.
    pub fn set_delay_crossfade(&mut self, target: usize) {
        let clamped = target.min(self.max_delay);
        if clamped == self.current_delay {
            return;
        }
        self.prev_delay = self.current_delay;
        self.current_delay = clamped;
        self.crossfade_pos = 0;
    }

    /// Clears ring memory without changing `current_delay` or allocating.
    pub fn reset(&mut self) {
        self.left.fill(0.0);
        self.right.fill(0.0);
        self.write_pos = 0;
        self.prev_delay = self.current_delay;
        self.crossfade_pos = self.crossfade_len;
    }

    #[inline]
    fn read_delayed(&self, buf: &[f32], delay: usize) -> f32 {
        let cap = buf.len();
        let idx = (self.write_pos + cap - delay) % cap;
        buf[idx]
    }

    /// Processes stereo buffers in-place with zero heap allocations.
    pub fn process_block(&mut self, left: &mut [f32], right: &mut [f32]) {
        if self.max_delay == 0 {
            return;
        }
        let frames = left.len().min(right.len());
        let cap = self.left.len();
        for i in 0..frames {
            self.left[self.write_pos] = left[i];
            self.right[self.write_pos] = right[i];

            let (out_l, out_r) = if self.crossfade_pos < self.crossfade_len {
                let alpha = (self.crossfade_pos + 1) as f32 / self.crossfade_len as f32;
                let inv = 1.0 - alpha;
                let pl = self.read_delayed(&self.left, self.prev_delay);
                let pr = self.read_delayed(&self.right, self.prev_delay);
                let cl = self.read_delayed(&self.left, self.current_delay);
                let cr = self.read_delayed(&self.right, self.current_delay);
                self.crossfade_pos += 1;
                if self.crossfade_pos >= self.crossfade_len {
                    self.prev_delay = self.current_delay;
                }
                (pl * inv + cl * alpha, pr * inv + cr * alpha)
            } else if self.current_delay == 0 {
                (left[i], right[i])
            } else {
                (
                    self.read_delayed(&self.left, self.current_delay),
                    self.read_delayed(&self.right, self.current_delay),
                )
            };

            left[i] = out_l;
            right[i] = out_r;
            self.write_pos = (self.write_pos + 1) % cap;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_two_strip_merge_compensates_dry_path_to_512_samples() {
        let master = 3;
        let plan = compute_pdc(&[
            PdcNode {
                id: 0,
                own_latency: 0,
                outputs: vec![master],
            },
            PdcNode {
                id: 16,
                own_latency: 512,
                outputs: vec![master],
            },
            PdcNode {
                id: master,
                own_latency: 0,
                outputs: vec![],
            },
        ])
        .unwrap();

        assert_eq!(plan.total_latency, 512);
        assert_eq!(plan.arrival_of(0), 0);
        assert_eq!(plan.arrival_of(16), 0);
        assert_eq!(plan.arrival_of(master), 512);
        assert_eq!(plan.delay_for_edge(0, master), 512);
        assert_eq!(plan.delay_for_edge(16, master), 0);
    }

    #[test]
    fn diamond_send_bus_dag_aligns_every_path_to_total_latency() {
        let nodes = vec![
            PdcNode {
                id: 16,
                own_latency: 100,
                outputs: vec![2, 20],
            },
            PdcNode {
                id: 17,
                own_latency: 400,
                outputs: vec![2],
            },
            PdcNode {
                id: 20,
                own_latency: 250,
                outputs: vec![2],
            },
            PdcNode {
                id: 2,
                own_latency: 64,
                outputs: vec![3],
            },
            PdcNode {
                id: 1,
                own_latency: 128,
                outputs: vec![3],
            },
            PdcNode {
                id: 3,
                own_latency: 32,
                outputs: vec![],
            },
        ];
        let plan = compute_pdc(&nodes).unwrap();
        // Longest path into node 2: 17 (400) -> 2 (400 + 64 = 464).
        // Parallel send 16 -> 20 -> 2: 100 + 250 = 350 -> edge 20->2 needs +50.
        // Direct 16 -> 2: 100 -> edge 16->2 needs +300.
        // Edge 1 -> 3: node 1 takes 128, node 2 finishes at 464 -> edge 1->3 needs +336.
        assert_eq!(plan.arrival_of(20), 100);
        assert_eq!(plan.arrival_of(2), 400);
        assert_eq!(plan.delay_for_edge(16, 20), 0);
        assert_eq!(plan.delay_for_edge(16, 2), 300);
        assert_eq!(plan.delay_for_edge(20, 2), 50);
        assert_eq!(plan.delay_for_edge(17, 2), 0);
        assert_eq!(plan.delay_for_edge(1, 3), 336);
        assert_eq!(plan.delay_for_edge(2, 3), 0);
        assert_eq!(plan.total_latency, 496);
    }

    #[test]
    fn cycle_returns_err() {
        let res = compute_pdc(&[
            PdcNode {
                id: 1,
                own_latency: 10,
                outputs: vec![2],
            },
            PdcNode {
                id: 2,
                own_latency: 10,
                outputs: vec![1],
            },
        ]);
        assert!(res.is_err());
    }

    #[test]
    fn delay_line_aligns_impulse_at_512_and_crossfades_to_256_smoothly() {
        let mut dl = DelayLine::new(1024, 48_000);
        dl.set_delay_immediate(512);
        let mut left = vec![0.0_f32; 1024];
        let mut right = vec![0.0_f32; 1024];
        left[0] = 1.0;
        right[0] = -0.5;
        dl.process_block(&mut left, &mut right);

        let peak_pos = left
            .iter()
            .position(|&s| (s - 1.0).abs() < 1e-6)
            .expect("impulse at sample 512");
        assert_eq!(peak_pos, 512);
        assert!((right[512] + 0.5).abs() < 1e-6);

        // Crossfade cleanly to 256 samples over ~10 ms (480 samples at 48 kHz) then verify exact 256 offset.
        dl.set_delay_crossfade(256);
        let mut xf_l = vec![0.25_f32; 512];
        let mut xf_r = vec![0.25_f32; 512];
        dl.process_block(&mut xf_l, &mut xf_r);
        assert!(xf_l.iter().all(|s| s.is_finite()));

        let mut test_l = vec![0.0_f32; 512];
        let mut test_r = vec![0.0_f32; 512];
        test_l[0] = 0.75;
        dl.process_block(&mut test_l, &mut test_r);
        assert!((test_l[256] - 0.75).abs() < 1e-6);
    }
}
