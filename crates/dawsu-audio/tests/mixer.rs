use dawsu_audio::{AudioClip, Mixer, ScheduledSound};
use std::sync::Arc;

fn clip(samples: Vec<f32>) -> Arc<AudioClip> {
    Arc::new(AudioClip {
        samples,
        channels: 1,
        sample_rate: 1000,
    })
}
#[test]
fn audio_clock_pause_seek_and_a_cue_use_output_frames() {
    let mut m = Mixer::new(1000);
    m.song = Some(clip(vec![0.2; 20]));
    m.cues = vec![ScheduledSound {
        at_ms: 2.0,
        end_ms: None,
        clip: clip(vec![0.3, 0.4]),
        gain: 1.0,
    }];
    m.playing = true;
    let mut output = [0.0; 4];
    m.render(&mut output, 1);
    assert_eq!(output, [0.2, 0.2, 0.5, 0.6]);
    assert_eq!(m.position_ms, 4.0);
    m.playing = false;
    m.render(&mut output, 1);
    assert_eq!(output, [0.0; 4]);
    assert_eq!(m.position_ms, 4.0);
    m.seek(2.0);
    m.playing = true;
    m.render(&mut output[..1], 1);
    assert_eq!(output[0], 0.5);
}
#[test]
fn final_output_is_bounded_and_loop_restarts_cues() {
    let mut m = Mixer::new(1000);
    m.song = Some(clip(vec![2.0; 10]));
    m.cycle = Some((0.0, 2.0));
    m.playing = true;
    let mut output = [0.0; 5];
    m.render(&mut output, 1);
    assert!(output.iter().all(|x| *x <= 1.0 && *x >= -1.0));
    assert_eq!(m.position_ms, 1.0);
}
#[test]
fn seeking_into_overlapping_and_looped_samples_reconstructs_active_audio() {
    let mut m = Mixer::new(1000);
    m.cues = vec![
        ScheduledSound {
            at_ms: 0.0,
            end_ms: None,
            clip: clip(vec![0.1; 10]),
            gain: 1.0,
        },
        ScheduledSound {
            at_ms: 2.0,
            end_ms: Some(8.0),
            clip: clip(vec![0.2, 0.3]),
            gain: 1.0,
        },
    ];
    m.seek(5.0);
    m.playing = true;
    let mut output = [0.0; 5];
    m.render(&mut output, 1);
    for (a, b) in output.iter().zip([0.4, 0.3, 0.4, 0.1, 0.1]) {
        assert!((a - b).abs() < 1e-6);
    }
}
