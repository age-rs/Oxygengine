extern crate oxygengine_utils as utils;

pub mod world_2d;
pub mod world_2d_climate_simulation;

pub mod prelude {
    pub use crate::world_2d::*;
    pub use crate::world_2d_climate_simulation::*;
}