#[derive(Debug, Clone)]
pub struct FreqBand {
    pub lo: f32,
    pub hi: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WeaponKind {
    Missile,
    Cannon,
    Both,
}

#[derive(Debug, Clone)]
pub struct Weapon {
    pub name:       String,
    pub kind:       WeaponKind,
    pub freq_bands: Vec<FreqBand>,
    pub min_amp:    f32,
    pub min_ratio:  f32,
}

#[derive(Debug, Clone)]
pub struct Mech {
    pub name:    String,
    pub weapons: Vec<Weapon>,
}

impl Mech {
    pub fn new(name: &str) -> Self {
        Mech {
            name:    name.to_string(),
            weapons: Vec::new(),
        }
    }

    pub fn add_weapon(mut self, weapon: Weapon) -> Self {
        self.weapons.push(weapon);
        self
    }

    pub fn missiles(&self) -> impl Iterator<Item = &Weapon> {
        self.weapons.iter().filter(|w| {
            w.kind == WeaponKind::Missile || w.kind == WeaponKind::Both
        })
    }

    pub fn cannons(&self) -> impl Iterator<Item = &Weapon> {
        self.weapons.iter().filter(|w| {
            w.kind == WeaponKind::Cannon || w.kind == WeaponKind::Both
        })
    }
}

pub struct MechRegistry {
    pub mechs: Vec<Mech>,
}

impl MechRegistry {
    pub fn new() -> Self {
        MechRegistry { mechs: Vec::new() }
    }

    pub fn register(mut self, mech: Mech) -> Self {
        self.mechs.push(mech);
        self
    }

    pub fn all_missile_bands(&self) -> Vec<&FreqBand> {
        self.mechs.iter()
            .flat_map(|m| m.missiles())
            .flat_map(|w| w.freq_bands.iter())
            .collect()
    }

    pub fn all_cannon_bands(&self) -> Vec<&FreqBand> {
        self.mechs.iter()
            .flat_map(|m| m.cannons())
            .flat_map(|w| w.freq_bands.iter())
            .collect()
    }

    pub fn match_missile(&self, warn_amp: f32, ratio: f32) -> Option<&Weapon> {
        self.mechs.iter()
            .flat_map(|m| m.missiles())
            .find(|w| warn_amp >= w.min_amp && ratio >= w.min_ratio)
    }

    pub fn match_cannon(&self, cannon_amp: f32, rms: f32) -> Option<&Weapon> {
        self.mechs.iter()
            .flat_map(|m| m.cannons())
            .find(|w| cannon_amp >= w.min_amp && rms >= 0.02)
    }
}