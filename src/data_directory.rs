pub fn create_all() {
    for dir in &[
        "models/character",
        "data/stego/missilesFrequencies",
        "data/stego/cannonFrequencies",
        "data/stego/mixFrequencies",
    ] {
        std::fs::create_dir_all(dir).expect("could not create directory");
    }
}