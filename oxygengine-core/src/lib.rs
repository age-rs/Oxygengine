#[cfg(feature = "parallel")]
extern crate rayon;
#[macro_use]
extern crate lazy_static;
extern crate typid;

#[macro_use]
pub mod log;

pub mod app;
pub mod assets;
pub mod error;
pub mod fetch;
pub mod hierarchy;
pub mod prefab;
pub mod state;

#[cfg(test)]
mod tests;

pub mod id {
    pub use typid::*;
}

pub mod ecs {
    pub use shred::Resource;
    pub use specs::*;
}

pub mod prelude {
    pub use crate::{
        app::*, assets::prelude::*, ecs::*, fetch::prelude::*, fetch::*, hierarchy::*, id::*,
        log::*, prefab::*, state::*,
    };
}
