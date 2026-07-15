use std::time::{Duration, Instant};

pub enum ThreatZone {
    Rear  { angle_deg: f32 },
    Flank { angle_deg: f32, left: bool },
}

#[derive(Default)]
pub struct ThreatState {
    pub alert:       Option<String>,
    pub alert_until: Option<Instant>,
    pub volume:      f32,
}

impl ThreatState {
    pub fn add_threat(&mut self, zone: ThreatZone, vol: f32) {
        let label = match &zone {
            ThreatZone::Rear { .. } => {
                "DANGER — REAR (6 o'clock)!".to_string()
            }
            ThreatZone::Flank { left, .. } => {
                let side = if *left { "LEFT FLANK" } else { "RIGHT FLANK" };
                format!("THREAT — {side}!")
            }
        };

        self.alert       = Some(label);
        self.alert_until = Some(Instant::now() + Duration::from_secs(2));
        self.volume      = vol;
    }
}