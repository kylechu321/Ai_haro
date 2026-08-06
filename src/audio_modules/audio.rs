use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::traits::{Consumer, Producer, Split};
use ringbuf::HeapRb;
use std::sync::{Arc, Mutex};
use crate::types::threat::{ThreatState, ThreatZone};
use crate::types::mech::{MechRegistry, WeaponKind};
use crate::audio_modules::frequency_adjustment::*;

pub fn run(state: Arc<Mutex<ThreatState>>, registry: Arc<MechRegistry>) {
    let host = cpal::default_host();

    let device = host.default_output_device()
        .expect("no output device found — check Windows sound settings");
    println!("Audio device: {}", device.name().unwrap_or_else(|_| "unknown".to_string()));

    let config = device.default_output_config()
        .expect("no output config found");
    println!("Sample rate: {} Hz | Channels: {}", config.sample_rate().0, config.channels());

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

    stream.play().expect("failed to start loopback stream");
    println!("Loopback stream started — listening for threats...");

    let mut buf         = vec![0f32; FFT_SIZE];
    let mut accumulator = Vec::<f32>::with_capacity(FFT_SIZE);

    // shazam rolling peak history (~1 second of history at 100ms/window)
    let num_bins = FFT_SIZE / 2;
    let mut peak_history: Vec<Vec<f32>> = Vec::with_capacity(PEAK_HISTORY_WINDOWS);

    // dtw rolling mfcc history
    let mut mfcc_history: Vec<Vec<f32>> = Vec::with_capacity(MFCC_WINDOW_FRAMES);

    // adaptive threshold state
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

        // apply Hann window
        let windowed: Vec<f32> = window.iter().enumerate().map(|(i, &s)| {
            let hann = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32
                / (FFT_SIZE - 1) as f32).cos());
            s * hann
        }).collect();

        let fft_mags        = compute_fft_magnitude(&windowed);
        let freq_resolution = sample_rate / FFT_SIZE as f32;

        // ── step 1: shazam recursive peak filtering ───────────────
        let neighborhood = 5;
        let mut this_window_peaks = vec![0.0f32; num_bins];
        for i in neighborhood..num_bins - neighborhood {
            let is_peak = (1..=neighborhood).all(|d| {
                fft_mags[i] > fft_mags[i - d] && fft_mags[i] > fft_mags[i + d]
            });
            if is_peak {
                this_window_peaks[i] = fft_mags[i];
            }
        }

        if peak_history.len() >= PEAK_HISTORY_WINDOWS { peak_history.remove(0); }
        peak_history.push(this_window_peaks);

        // keep only peaks consistent across rolling history
        let min_consistency = 0.30;
        let history_len     = peak_history.len() as f32;
        let mut clean_spectrum = vec![0.0f32; num_bins];
        for bin in 0..num_bins {
            let times_present = peak_history.iter()
                .filter(|w| w[bin] > 0.0)
                .count() as f32;
            let consistency = times_present / history_len;
            if consistency >= min_consistency {
                let mut amps: Vec<f32> = peak_history.iter()
                    .map(|w| w[bin])
                    .filter(|&a| a > 0.0)
                    .collect();
                if !amps.is_empty() {
                    amps.sort_by(|a, b| a.partial_cmp(b).unwrap());
                    clean_spectrum[bin] = amps[amps.len() / 2];
                }
            }
        }

        // ── step 2: mfcc extraction from cleaned spectrum ─────────
        let mfcc_frame = compute_live_mfcc(&clean_spectrum, sample_rate);

        if mfcc_history.len() >= MFCC_WINDOW_FRAMES { mfcc_history.remove(0); }
        mfcc_history.push(mfcc_frame);

        if mfcc_history.len() < 3 { continue; }

        // ── step 3: dtw recursive classification ──────────────────
        let templates = registry.all_templates();
        if !templates.is_empty() {
            let best_match = templates.iter()
                .map(|(template, kind)| {
                    let dist = dtw_distance(&mfcc_history, template);
                    (dist, kind)
                })
                .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

            if let Some((dist, kind)) = best_match {
                if dist < DTW_THRESHOLD {
                    match kind {
                        WeaponKind::Missile => {
                            println!("DTW match: MISSILE detected (dist={dist:.1})");
                            if let Ok(mut s) = state.lock() {
                                s.add_threat(ThreatZone::Rear { angle_deg: 180.0 }, rms);
                            }
                        }
                        WeaponKind::Cannon => {
                            println!("DTW match: CANNON detected (dist={dist:.1})");
                            let left = rand_f32() < 0.5;
                            let a    = if left { 90.0 + rand_f32() * 45.0 }
                            else    { 225.0 + rand_f32() * 45.0 };
                            if let Ok(mut s) = state.lock() {
                                s.add_threat(ThreatZone::Flank { angle_deg: a, left }, rms);
                            }
                        }
                        WeaponKind::Ignore => {} _ => {}
                    }
                }
            }
        }

        // ── step 4: fallback adaptive threshold detection ─────────
        let band_max = |lo: f32, hi: f32| -> f32 {
            let lo_bin = (lo / freq_resolution) as usize;
            let hi_bin = ((hi / freq_resolution) as usize).min(fft_mags.len() - 1);
            fft_mags[lo_bin..=hi_bin].iter().cloned().fold(0.0_f32, f32::max)
        };

        let warn_amp: f32 = registry.all_missile_bands()
            .iter()
            .map(|b| band_max(b.lo, b.hi))
            .fold(0.0_f32, f32::max);

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
            println!("Fallback: MISSILE detected (warn_amp={warn_amp:.1} ratio={ratio:.2})");
            if let Ok(mut s) = state.lock() {
                s.add_threat(ThreatZone::Rear { angle_deg: 180.0 }, rms);
            }
        } else if registry.match_cannon(cannon_amp, rms).is_some()
            && cannon_amp > warn_amp * 2.0
        {
            println!("Fallback: CANNON detected (cannon_amp={cannon_amp:.1})");
            let left = rand_f32() < 0.5;
            let a    = if left { 90.0 + rand_f32() * 45.0 } else { 225.0 + rand_f32() * 45.0 };
            if let Ok(mut s) = state.lock() {
                s.add_threat(ThreatZone::Flank { angle_deg: a, left }, rms);
            }
        }

        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

pub fn dtw_distance(a: &[Vec<f32>], b: &[Vec<f32>]) -> f32 {
    let n = a.len();
    let m = b.len();
    if n == 0 || m == 0 { return f32::MAX; }

    let mut cost     = vec![vec![f32::MAX; m]; n];
    cost[0][0]       = euclidean_distance(&a[0], &b[0]);

    for i in 1..n {
        cost[i][0] = cost[i-1][0] + euclidean_distance(&a[i], &b[0]);
    }
    for j in 1..m {
        cost[0][j] = cost[0][j-1] + euclidean_distance(&a[0], &b[j]);
    }
    for i in 1..n {
        for j in 1..m {
            let d      = euclidean_distance(&a[i], &b[j]);
            cost[i][j] = d + cost[i-1][j].min(cost[i][j-1]).min(cost[i-1][j-1]);
        }
    }

    cost[n-1][m-1]
}

fn euclidean_distance(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter())
        .map(|(x, y)| (x - y).powi(2))
        .sum::<f32>()
        .sqrt()
}

fn compute_live_mfcc(clean_spectrum: &[f32], sample_rate: f32) -> Vec<f32> {
    let num_mel_filters = 26;
    let num_mfcc        = 13;
    let num_bins        = clean_spectrum.len();

    let mel_filters = build_mel_filters(num_mel_filters, num_bins, sample_rate as u32);

    let mel_spectrum: Vec<f32> = mel_filters.iter()
        .map(|filter| {
            filter.iter().enumerate()
                .map(|(i, &w)| w * clean_spectrum[i].max(1e-10))
                .sum::<f32>()
                .max(1e-10)
                .ln()
        })
        .collect();

    (0..num_mfcc)
        .map(|n| {
            mel_spectrum.iter().enumerate()
                .map(|(m, &s)| {
                    s * (std::f32::consts::PI * n as f32
                        * (2 * m + 1) as f32
                        / (2 * num_mel_filters) as f32).cos()
                })
                .sum::<f32>()
                * (2.0 / num_mel_filters as f32).sqrt()
        })
        .collect()
}

fn build_mel_filters(num_filters: usize, num_bins: usize, sample_rate: u32) -> Vec<Vec<f32>> {
    let hz_to_mel = |hz: f32| 2595.0 * (1.0 + hz / 700.0).log10();
    let mel_to_hz = |mel: f32| 700.0 * (10.0_f32.powf(mel / 2595.0) - 1.0);

    let low_mel  = hz_to_mel(0.0);
    let high_mel = hz_to_mel(sample_rate as f32 / 2.0);

    let mel_points: Vec<f32> = (0..=num_filters + 1)
        .map(|i| mel_to_hz(
            low_mel + i as f32 * (high_mel - low_mel) / (num_filters + 1) as f32
        ))
        .collect();

    let freq_resolution = sample_rate as f32 / (num_bins * 2) as f32;
    let bin_points: Vec<usize> = mel_points.iter()
        .map(|&hz| (hz / freq_resolution) as usize)
        .collect();

    (0..num_filters).map(|m| {
        let mut filter = vec![0.0f32; num_bins];
        for k in bin_points[m]..bin_points[m + 1] {
            if k < num_bins {
                filter[k] = (k - bin_points[m]) as f32
                    / (bin_points[m + 1] - bin_points[m]) as f32;
            }
        }
        for k in bin_points[m + 1]..bin_points[m + 2] {
            if k < num_bins {
                filter[k] = (bin_points[m + 2] - k) as f32
                    / (bin_points[m + 2] - bin_points[m + 1]) as f32;
            }
        }
        filter
    }).collect()
}

pub fn compute_fft_magnitude(samples: &[f32]) -> Vec<f32> {
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