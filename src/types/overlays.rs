pub struct OverlayConfig {
    pub x:      i32,
    pub y:      i32,
    pub width:  i32,
    pub height: i32,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        OverlayConfig { x: 100, y: 800, width: 150, height: 150 }
    }
}