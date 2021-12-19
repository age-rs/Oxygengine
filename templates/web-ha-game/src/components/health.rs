use oxygengine::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Ignite, Debug, Default, Copy, Clone, Serialize, Deserialize)]
pub struct Health(pub usize);

impl Prefab for Health {}
impl PrefabComponent for Health {}
