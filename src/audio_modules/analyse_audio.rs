#![allow(dead_code)]

use std::fs;
use std::process::Command;
use rustfft::num_complex::Complex;
use crate::audio_modules::frequency_adjustment::*;
use dotenvy::dotenv;
use std::env;

pub fn analyse_and_register(mech_name: &str, weapon_kind: &str) {
    dotenv().ok();
    let base_path = match weapon_kind {
        "missile" => env::var("STEGO_MISSILE_PATH")
            .unwrap_or_else(|_| "datas/stego/missileFrequencies".to_string()),
        "cannon"  => env::var("STEGO_CANNON_PATH")
            .unwrap_or_else(|_| "datas/stego/cannonFrequencies".to_string()),
        "both"    => env::var("STEGO_MIX_PATH")
            .unwrap_or_else(|_| "datas/stego/mixFrequencies".to_string()),
        _         => env::var("STEGO_IGNORE_PATH")
            .unwrap_or_else(|_| "datas/stego/ignore".to_string()),
    };
    let mp4_path = format!("{base_path}/{mech_name}_{weapon_kind}.mp4");
    println!("Reading from: {base_path}/...");

    let wav_path = format!("{base_path}/temp_{mech_name}.wav");
    extract_audio(&mp4_path, &wav_path);

    let samples        = load_wav(&wav_path);
    let clean_spectrum = shazam_peak_filter(&samples);
    let mfcc_template  = extract_mfcc_template(&clean_spectrum);
    let bands          = find_persistent_bands(&samples, weapon_kind);

    println!("MFCC template: {} frames", mfcc_template.len());
    for (lo, hi, score) in &bands {
        println!("  {lo:.0}–{hi:.0} Hz  (score={score:.2})");
    }

    append_to_registry(mech_name, weapon_kind, &bands, &mfcc_template);
    let _ = std::fs::remove_file(&wav_path);
    println!("registry.rs updated with {mech_name} {weapon_kind}");
}

fn shazam_peak_filter(samples: &[f32]) -> Vec<f32> {
    let num_bins  = FFT_SIZE / 2;
    let hop       = FFT_SIZE / 2;

    let mut peak_count    = vec![0u32; num_bins];
    let mut peak_amp      = vec![0.0f32; num_bins];
    let mut total_windows = 0u32;

    let mut planner = rustfft::FftPlanner::<f32>::new();
    let fft         = planner.plan_fft_forward(FFT_SIZE);

    for chunk_start in (0..samples.len().saturating_sub(FFT_SIZE)).step_by(hop) {
        let chunk = &samples[chunk_start..chunk_start + FFT_SIZE];
        let rms: f32 = {
            let sum: f32 = chunk.iter().map(|s| s * s).sum();
            (sum / FFT_SIZE as f32).sqrt()
        };
        if rms < 0.005 { continue; }

        let mut buffer: Vec<Complex<f32>> = chunk.iter().enumerate().map(|(i, &s)| {
            let hann = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32
                / (FFT_SIZE - 1) as f32).cos());
            Complex { re: s * hann, im: 0.0 }
        }).collect();

        fft.process(&mut buffer);

        let amps: Vec<f32> = buffer[..num_bins].iter().map(|c| c.norm()).collect();

        let neighborhood = 5;
        for i in neighborhood..num_bins - neighborhood {
            let is_peak = (1..=neighborhood).all(|d| {
                amps[i] > amps[i - d] && amps[i] > amps[i + d]
            });
            if is_peak {
                peak_count[i] += 1;
                if amps[i] > peak_amp[i] { peak_amp[i] = amps[i]; }
            }
        }

        total_windows += 1;
    }

    if total_windows == 0 { return vec![0.0; num_bins]; }

    let min_consistency = 0.25;
    let mut clean = vec![0.0f32; num_bins];
    for i in 0..num_bins {
        let consistency = peak_count[i] as f32 / total_windows as f32;
        if consistency >= min_consistency {
            clean[i] = peak_amp[i];
        }
    }

    println!("Shazam: {total_windows} windows, {} consistent peaks kept",
             clean.iter().filter(|&&a| a > 0.0).count());

    clean
}

fn extract_mfcc_template(clean_spectrum: &[f32]) -> Vec<Vec<f32>> {
    let num_mel_filters = 26;
    let num_mfcc        = 13;
    let num_bins        = clean_spectrum.len();

    let mel_filters  = build_mel_filters(num_mel_filters, num_bins, SAMPLE_RATE);
    let mel_spectrum: Vec<f32> = mel_filters.iter()
        .map(|filter| {
            filter.iter().enumerate()
                .map(|(i, &w)| w * clean_spectrum[i].max(1e-10))
                .sum::<f32>()
                .max(1e-10)
                .ln()
        })
        .collect();

    let mfcc: Vec<f32> = (0..num_mfcc)
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
        .collect();

    vec![mfcc]
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

fn find_persistent_bands(samples: &[f32], weapon_kind: &str) -> Vec<(f32, f32, f32)> {
    let freq_resolution = SAMPLE_RATE as f32 / FFT_SIZE as f32;
    let num_bins        = FFT_SIZE / 2;
    let hop             = FFT_SIZE / 2;
    let mut amp_per_window: Vec<Vec<f32>> = vec![Vec::new(); num_bins];

    let mut planner = rustfft::FftPlanner::<f32>::new();
    let fft         = planner.plan_fft_forward(FFT_SIZE);

    for chunk_start in (0..samples.len().saturating_sub(FFT_SIZE)).step_by(hop) {
        let chunk = &samples[chunk_start..chunk_start + FFT_SIZE];
        let rms: f32 = {
            let sum: f32 = chunk.iter().map(|s| s * s).sum();
            (sum / FFT_SIZE as f32).sqrt()
        };
        if rms < 0.005 { continue; }

        let mut buffer: Vec<Complex<f32>> = chunk.iter().enumerate().map(|(i, &s)| {
            let hann = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32
                / (FFT_SIZE - 1) as f32).cos());
            Complex { re: s * hann, im: 0.0 }
        }).collect();

        fft.process(&mut buffer);
        for (i, c) in buffer[..num_bins].iter().enumerate() {
            amp_per_window[i].push(c.norm());
        }
    }

    let total_windows = amp_per_window[0].len();
    if total_windows < 3 { return Vec::new(); }

    let median_amps: Vec<f32> = amp_per_window.iter().map(|wins| {
        let mut sorted = wins.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mid = sorted.len() / 2;
        if sorted.len() % 2 == 0 {
            (sorted[mid - 1] + sorted[mid]) / 2.0
        } else {
            sorted[mid]
        }
    }).collect();

    let std_devs: Vec<f32> = amp_per_window.iter().enumerate().map(|(i, wins)| {
        let mean     = median_amps[i];
        let variance = wins.iter().map(|&a| (a - mean).powi(2)).sum::<f32>()
            / wins.len() as f32;
        variance.sqrt()
    }).collect();

    let (min_freq, max_freq): (f32, f32) = match weapon_kind {
        "missile" => (200.0, 8000.0),
        "cannon"  => (20.0,  500.0),
        _         => (20.0,  8000.0),
    };

    let min_bin = (min_freq / freq_resolution) as usize;
    let max_bin = (max_freq / freq_resolution) as usize;

    let mut scored: Vec<(usize, f32)> = (min_bin..max_bin.min(num_bins))
        .filter_map(|i| {
            let median = median_amps[i];
            let std    = std_devs[i];
            if median < 1.0 { return None; }
            let cv    = std / (median + 1e-6);
            let score = median / (cv + 0.1);
            Some((i, score))
        })
        .collect();

    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

    let mut bands:     Vec<(f32, f32, f32)> = Vec::new();
    let mut used_bins: Vec<usize>            = Vec::new();
    let half_bins = (BAND_WIDTH_HZ / 2.0 / freq_resolution) as usize;

    for (bin, score) in scored.iter().take(200) {
        let too_close = used_bins.iter()
            .any(|&b| (*bin as isize - b as isize).unsigned_abs() < half_bins * 2);
        if !too_close {
            let freq = *bin as f32 * freq_resolution;
            let lo   = (freq - BAND_WIDTH_HZ / 2.0).max(0.0);
            let hi   = freq + BAND_WIDTH_HZ / 2.0;
            bands.push((lo, hi, *score));
            used_bins.push(*bin);
        }
        if bands.len() >= TOP_BANDS { break; }
    }

    bands
}

fn extract_audio(mp4: &str, wav: &str) {
    Command::new("ffmpeg")
        .args(["-y", "-i", mp4, "-vn",
            "-ar", &SAMPLE_RATE.to_string(),
            "-ac", "1", "-f", "wav", wav])
        .status()
        .expect("ffmpeg not found — make sure ffmpeg is on PATH");
}

fn load_wav(path: &str) -> Vec<f32> {
    let mut reader = hound::WavReader::open(path).expect("could not open wav");
    let spec       = reader.spec();
    match spec.sample_format {
        hound::SampleFormat::Float => {
            reader.samples::<f32>().map(|s| s.unwrap()).collect()
        }
        hound::SampleFormat::Int => {
            let max = (1i32 << (spec.bits_per_sample - 1)) as f32;
            reader.samples::<i32>().map(|s| s.unwrap() as f32 / max).collect()
        }
    }
}

fn append_to_registry(
    mech_name:     &str,
    weapon_kind:   &str,
    bands:         &[(f32, f32, f32)],
    mfcc_template: &[Vec<f32>],
) {
    let weapon_kind_rust = match weapon_kind {
        "missile" => "WeaponKind::Missile",
        "cannon"  => "WeaponKind::Cannon",
        "ignore"  => "WeaponKind::Ignore",
        _         => "WeaponKind::Both",
    };

    let min_ratio = if weapon_kind == "missile" { MIN_AMP_RATIO } else { 0.0 };
    let min_amp   = bands.first().map(|(_, _, s)| s * 0.6).unwrap_or(8.0);

    let freq_bands: String = bands.iter()
        .map(|(lo, hi, _)| format!(
            "                    FreqBand {{ lo: {lo:.1}, hi: {hi:.1} }}"
        ))
        .collect::<Vec<_>>()
        .join(",\n");

    let mfcc_str: String = mfcc_template.iter()
        .map(|frame| {
            let coeffs = frame.iter()
                .map(|c| format!("{c:.6}"))
                .collect::<Vec<_>>()
                .join(", ");
            format!("                        vec![{coeffs}]")
        })
        .collect::<Vec<_>>()
        .join(",\n");

    let new_entry = format!(
        r#"        .register(
            Mech::new("{mech_name}")
                .add_weapon(Weapon {{
                    name:       "{mech_name} {weapon_kind}".to_string(),
                    kind:       {weapon_kind_rust},
                    freq_bands: vec![
{freq_bands}
                    ],
                    min_amp:    {min_amp:.1},
                    min_ratio:  {min_ratio:.2},
                    mfcc_template: vec![
{mfcc_str}
                    ],
                }})
        )"#
    );

    let current = fs::read_to_string("src/registry.rs").unwrap_or_default();
    let updated = if let Some(pos) = current.rfind('}') {
        format!("{}\n{}\n}}", &current[..pos].trim_end(), new_entry)
    } else {
        current
    };

    fs::write("src/registry.rs", updated).expect("could not write registry.rs");
}