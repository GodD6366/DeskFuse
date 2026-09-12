/// Primary-display extent and global origin in platform logical desktop coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MainDisplayGeometry {
    pub width: u32,
    pub height: u32,
    pub origin_x: f64,
    pub origin_y: f64,
}
