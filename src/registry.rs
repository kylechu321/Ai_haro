use crate::types::mech::{FreqBand, Mech, MechRegistry, Weapon, WeaponKind};

pub fn build() -> MechRegistry {
    MechRegistry::new()
        .register(
            Mech::new("Stego")
                .add_weapon(Weapon {
                    name:       "Stego Missile".to_string(),
                    kind:       WeaponKind::Missile,
                    freq_bands: vec![FreqBand { lo: 340.0, hi: 410.0 }],
                    min_amp:    8.0,
                    min_ratio:  1.10,
                })
                .add_weapon(Weapon {
                    name:       "Stego Cannon".to_string(),
                    kind:       WeaponKind::Cannon,
                    freq_bands: vec![FreqBand { lo: 40.0, hi: 160.0 }],
                    min_amp:    8.0,
                    min_ratio:  0.0,
                })
        )
}
