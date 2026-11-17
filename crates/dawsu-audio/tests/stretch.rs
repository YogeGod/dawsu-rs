use dawsu_audio::{AudioClip, stretch};

#[test]
fn half_speed_doubles_duration_while_retaining_a_sine_tones_pitch() {
    let rate = 16000;
    let input = AudioClip {
        samples: (0..rate * 2)
            .map(|i| (2.0 * std::f32::consts::PI * 220.0 * i as f32 / rate as f32).sin() * 0.5)
            .collect(),
        channels: 1,
        sample_rate: rate,
    };
    let output = stretch(&input, 0.5).unwrap();
    assert_eq!(output.samples.len(), input.samples.len() * 2);
    let section = &output.samples[rate as usize..rate as usize * 3];
    let crossings = section
        .windows(2)
        .filter(|p| p[0] <= 0.0 && p[1] > 0.0)
        .count();
    let frequency = crossings as f64 / 2.0;
    assert!(
        (215.0..=225.0).contains(&frequency),
        "measured zero-crossing frequency: {frequency}"
    );
    assert!(output.samples.iter().all(|v| v.is_finite()));
}
