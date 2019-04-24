#[cfg(feature = "web")]
extern crate oxygengine_backend_web;
#[cfg(feature = "composite-renderer")]
extern crate oxygengine_composite_renderer;
#[cfg(feature = "web")]
extern crate oxygengine_composite_renderer_backend_web;
extern crate oxygengine_core;
#[cfg(feature = "input")]
extern crate oxygengine_input;

pub mod platform;

pub mod core {
    pub use oxygengine_core::*;
}
#[cfg(feature = "input")]
pub mod input {
    pub use oxygengine_input::*;
}
pub mod backend {
    #[cfg(feature = "web")]
    pub mod web {
        pub use oxygengine_backend_web::app::*;
        pub use oxygengine_backend_web::fetch::engines::web::*;
        pub use oxygengine_backend_web::*;
        #[cfg(feature = "composite-renderer")]
        pub use oxygengine_composite_renderer_backend_web::*;
    }
}
#[cfg(feature = "composite-renderer")]
pub mod composite_renderer {
    pub use oxygengine_composite_renderer::*;
}

pub mod prelude {
    pub use crate::core::prelude::*;
    pub use crate::platform::*;
}
