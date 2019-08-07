use crate::{
    resource::{NavMesh, NavMeshesRes, NavPathMode, NavQuery, NavVec3, ZERO_TRESHOLD},
    Scalar,
};
use core::{
    ecs::{Component, VecStorage},
    id::ID,
};

#[derive(Debug, Clone)]
pub struct NavAgent {
    id: ID<NavAgent>,
    pub position: NavVec3,
    pub direction: NavVec3,
    pub speed: Scalar,
    pub min_target_distance: Scalar,
    destination: Option<(NavVec3, NavQuery, NavPathMode, ID<NavMesh>)>,
    path: Option<Vec<NavVec3>>,
    dirty_path: bool,
}

impl Component for NavAgent {
    type Storage = VecStorage<Self>;
}

impl Default for NavAgent {
    fn default() -> Self {
        Self::new(Default::default())
    }
}

impl NavAgent {
    pub fn new(position: NavVec3) -> Self {
        Self::new_with_direction(position, Default::default())
    }

    pub fn new_with_direction(position: NavVec3, direction: NavVec3) -> Self {
        Self {
            id: ID::default(),
            position,
            direction: direction.normalize(),
            speed: 10.0,
            min_target_distance: 1.0,
            destination: None,
            path: None,
            dirty_path: false,
        }
    }

    pub fn id(&self) -> ID<NavAgent> {
        self.id
    }

    pub fn destination(&self) -> Option<NavVec3> {
        if let Some((destination, _, _, _)) = &self.destination {
            Some(*destination)
        } else {
            None
        }
    }

    pub fn set_destination(
        &mut self,
        point: NavVec3,
        query: NavQuery,
        mode: NavPathMode,
        mesh: ID<NavMesh>,
    ) {
        self.destination = Some((point, query, mode, mesh));
        self.dirty_path = true;
    }

    pub fn clear_path(&mut self) {
        self.destination = None;
        self.dirty_path = false;
        self.path = None;
    }

    pub fn path(&self) -> Option<&[NavVec3]> {
        if let Some(path) = &self.path {
            Some(path)
        } else {
            None
        }
    }

    pub fn destination_reached(&self) -> bool {
        if let Some((destination, _, _, _)) = &self.destination {
            (self.position - *destination).sqr_magnitude() < ZERO_TRESHOLD
        } else {
            true
        }
    }

    pub fn process(&mut self, meshes: &NavMeshesRes, delta_time: Scalar) {
        if self.dirty_path {
            self.dirty_path = false;
            if let Some((destination, query, mode, id)) = self.destination {
                if let Some(mesh) = meshes.0.get(&id) {
                    self.path = mesh.find_path(self.position, destination, query, mode);
                } else {
                    self.destination = None;
                }
            }
        }
        if delta_time < 0.0 {
            return;
        }
        if let Some(path) = &self.path {
            let target = Self::target_point(
                path,
                self.position,
                self.speed.max(self.min_target_distance) * delta_time,
            )
            .0;
            let diff = target - self.position;
            let dir = diff.normalize();
            self.position = self.position + dir * (self.speed * delta_time).min(diff.magnitude());
            self.direction = diff.normalize();
        }
    }

    pub fn target_point(path: &[NavVec3], point: NavVec3, offset: Scalar) -> (NavVec3, Scalar) {
        match path.len() {
            0 => (point, 0.0),
            1 => (path[0], 0.0),
            2 => Self::point_on_line(path[0], path[1], point, offset),
            _ => path
                .windows(2)
                .scan(0.0, |state, pair| {
                    let s = *state;
                    *state += (pair[1] - pair[0]).magnitude();
                    Some((s, pair))
                })
                .map(|(dist, pair)| {
                    let (p, d) = Self::point_on_line(pair[0], pair[1], point, offset);
                    (p, dist + d)
                })
                .min_by(|(_, a), (_, b)| b.partial_cmp(&a).unwrap())
                .unwrap(),
        }
    }

    fn point_on_line(
        from: NavVec3,
        to: NavVec3,
        point: NavVec3,
        offset: Scalar,
    ) -> (NavVec3, Scalar) {
        let d = (to - from).magnitude();
        if d < ZERO_TRESHOLD {
            return (from, 0.0);
        }
        let p = point.project(from, to) + offset / d;
        if p <= 0.0 {
            (from, 0.0)
        } else if p >= 1.0 {
            (to, d)
        } else {
            (NavVec3::unproject(from, to, p), p * d)
        }
    }
}
