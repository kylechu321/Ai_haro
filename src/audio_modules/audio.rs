use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::traits::{Consumer, Producer, Split};
use ringbuf::HeapRb;
use std::sync::{Arc, Mutex};
use crate::types::threat::{ThreatState, ThreatZone};
use crate::types::mech::MechRegistry;

// adaptive threshold tuning
const HISTORY_LEN:      usize = 50;
const SPIKE_SIGMA:      f32   = 1.5;
const WARN_AMP_FLOOR:   f32   = 8.0;
const WARN_AMP_CEILING: f32   = 50.0;
const RATIO_FLOOR:      f32   = 1.10;
const RATIO_CEILING:    f32   = 2.0;

pub fn run(state: Arc<Mutex<ThreatState>>, registry: Arc<MechRegistry>) {
    let host        = cpal::default_host();
    let device      = host.default_output_device().expect("no output device");
    let config      = device.default_output_config().expect("no output config");
    let channels    = config.channels() as usize;
    let sample_rate = config.sample_rate().0 as f32;
    let stream_cfg: cpal::StreamConfig = config.into();

    let rb = HeapRb::<f32>::new(8192);
    let (mut prod, mut cons) = rb.split();

    let stream = device.build_input_stream(
        &stream_cfg,
        move |data: &[f32], _| {
            let mono: Vec<f32> = data.chunks(channels)
                .map(|frame| frame.iter().sum::<f32>() / channels as f32)
                .collect();
            for s in mono {
                let _ = prod.try_push(s);
            }
        },
        |e| eprintln!("loopback error: {e}"),
        None,
    ).expect("failed to build loopback stream");

    stream.play().expect("failed to start stream");

    const FFT_SIZE: usize = 4096;
    let mut buf         = vec![0f32; FFT_SIZE];
    let mut accumulator = Vec::<f32>::with_capacity(FFT_SIZE);

    let mut warn_history:   Vec<f32> = Vec::with_capacity(HISTORY_LEN);
    let mut cannon_history: Vec<f32> = Vec::with_capacity(HISTORY_LEN);

    loop {
        let n = cons.pop_slice(&mut buf);
        if n == 0 {
            std::thread::sleep(std::time::Duration::from_millis(5));
            continue;
        }

        accumulator.extend_from_slice(&buf[..n]);
        if accumulator.len() < FFT_SIZE { continue; }

        let window: Vec<f32> = accumulator.drain(..FFT_SIZE).collect();

        let rms: f32 = {
            let sum: f32 = window.iter().map(|s| s * s).sum();
            (sum / FFT_SIZE as f32).sqrt()
        };

        let windowed: Vec<f32> = window.iter().enumerate().map(|(i, &s)| {
            let hann = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32
                / (FFT_SIZE - 1) as f32).cos());
            s * hann
        }).collect();

        let fft_mags        = compute_fft_magnitude(&windowed);
        let freq_resolution = sample_rate / FFT_SIZE as f32;

        let band_max = |lo: f32, hi: f32| -> f32 {
            let lo_bin = (lo / freq_resolution) as usize;
            let hi_bin = ((hi / freq_resolution) as usize).min(fft_mags.len() - 1);
            fft_mags[lo_bin..=hi_bin].iter().cloned().fold(0.0_f32, f32::max)
        };

        // check all missile bands across all registered mechs
        let warn_amp: f32 = registry.all_missile_bands()
            .iter()
            .map(|b| band_max(b.lo, b.hi))
            .fold(0.0_f32, f32::max);

        // check all cannon bands across all registered mechs
        let cannon_amp: f32 = registry.all_cannon_bands()
            .iter()
            .map(|b| band_max(b.lo, b.hi))
            .fold(0.0_f32, f32::max);

        if warn_history.len() >= HISTORY_LEN   { warn_history.remove(0); }
        if cannon_history.len() >= HISTORY_LEN { cannon_history.remove(0); }
        warn_history.push(warn_amp);
        cannon_history.push(cannon_amp);

        if warn_history.len() < 10 { continue; }

        let warn_mean = warn_history.iter().sum::<f32>() / warn_history.len() as f32;
        let warn_std  = {
            let variance = warn_history.iter()
                .map(|x| (x - warn_mean).powi(2))
                .sum::<f32>() / warn_history.len() as f32;
            variance.sqrt()
        };

        let cannon_mean = cannon_history.iter().sum::<f32>() / cannon_history.len() as f32;

        let dynamic_warn_threshold = (warn_mean + SPIKE_SIGMA * warn_std)
            .clamp(WARN_AMP_FLOOR, WARN_AMP_CEILING);

        let dynamic_ratio = (warn_mean / (cannon_mean + 1e-6))
            .clamp(RATIO_FLOOR, RATIO_CEILING);

        let ratio = warn_amp / (cannon_amp + 1e-6);

        if registry.match_missile(warn_amp, ratio).is_some()
            && warn_amp > dynamic_warn_threshold
            && ratio   > dynamic_ratio
        {
            if let Ok(mut s) = state.lock() {
                s.add_threat(ThreatZone::Rear { angle_deg: 180.0 }, rms);
            }
        } else if registry.match_cannon(cannon_amp, rms).is_some()
            && cannon_amp > warn_amp * 2.0
        {
            let left = rand_f32() < 0.5;
            let a    = if left { 90.0 + rand_f32() * 45.0 } else { 225.0 + rand_f32() * 45.0 };
            if let Ok(mut s) = state.lock() {
                s.add_threat(ThreatZone::Flank { angle_deg: a, left }, rms);
            }
        }

        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn compute_fft_magnitude(samples: &[f32]) -> Vec<f32> {
    use rustfft::{FftPlanner, num_complex::Complex};
    let mut planner = FftPlanner::<f32>::new();
    let fft         = planner.plan_fft_forward(samples.len());
    let mut buffer: Vec<Complex<f32>> = samples.iter()
        .map(|&s| Complex { re: s, im: 0.0 })
        .collect();
    fft.process(&mut buffer);
    buffer[..samples.len() / 2].iter().map(|c| c.norm()).collect()
}

fn rand_f32() -> f32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    (nanos % 1000) as f32 / 1000.0
}