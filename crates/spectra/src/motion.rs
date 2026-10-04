//! How the disc moves, ported from Rainbow Player's `discScene.ts`.
//!
//! One change of principle: Rainbow Player flips its disc end over end for as
//! long as the page is open. Here it shows off both faces twice on arrival,
//! then settles label-forward and goes completely still - nothing is drawn,
//! and the GPU idles, until something happens. It spins while playing, the
//! way a disc does in a drive.

/// A half-turn, eased.
const FLIP_SECONDS: f32 = 1.15;
/// How long each face is held to camera between flips.
const HOLD_SECONDS: f32 = 2.1;
/// Flips on arrival: label, read side, label.
const SHOWCASE_FLIPS: u8 = 2;
const PLAYING_RPM: f32 = 320.0;
/// A disc on the shelf that is not in focus, against one that is.
const SHELF_REST: f32 = 0.84;

#[derive(Debug, Clone, Copy, Default)]
pub struct Pose {
    /// About the vertical axis: flips, the three-quarter turn, sway.
    pub turn: f32,
    /// Leaning back.
    pub tilt: f32,
    /// About the disc's own axis.
    pub spin: f32,
    /// 0 invisible to 1 fully present.
    pub presence: f32,
    /// Size against a disc that fills its box: smaller at rest on the shelf.
    pub zoom: f32,
}

pub struct Motion {
    flip_from: f32,
    flip_to: f32,
    flip_t: f32,
    hold: f32,
    flips_left: u8,
    sway_t: f32,
    sway: f32,
    yaw: f32,
    tilt: f32,
    spin: f32,
    rpm: f32,
    spinning: bool,
    presence: f32,
    zoom: f32,
    zoom_to: f32,
    reduced: bool,
}

/// Critically damped approach: no overshoot, frame-rate independent.
fn damp(current: f32, target: f32, lambda: f32, dt: f32) -> f32 {
    current + (target - current) * (1.0 - (-lambda * dt).exp())
}

/// Damp, and snap once close enough that nobody could see the difference -
/// otherwise an approach never finishes and the frame loop never stops.
fn settle(value: &mut f32, target: f32, lambda: f32, dt: f32, epsilon: f32) {
    *value = damp(*value, target, lambda, dt);
    if (*value - target).abs() < epsilon {
        *value = target;
    }
}

fn ease_in_out_cubic(t: f32) -> f32 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

impl Motion {
    pub fn new(reduced: bool) -> Self {
        Self {
            flip_from: 0.0,
            flip_to: 0.0,
            flip_t: 1.0,
            hold: 0.6,
            flips_left: if reduced { 0 } else { SHOWCASE_FLIPS },
            sway_t: 0.0,
            sway: 0.0,
            yaw: 0.0,
            tilt: 0.0,
            spin: 0.0,
            rpm: 0.0,
            spinning: false,
            presence: 0.0,
            zoom: 1.0,
            zoom_to: 1.0,
            reduced,
        }
    }

    /// A disc on the library's shelf: it fades in at rest, label side out,
    /// and smaller than the one in focus.
    pub fn shelved(reduced: bool) -> Self {
        let mut motion = Self::new(reduced);
        motion.flips_left = 0;
        motion.zoom = SHELF_REST;
        motion.zoom_to = SHELF_REST;
        motion
    }

    /// The shelf's focus arrived or left. The disc that has just been
    /// reached turns straight away, rather than sitting through a hold first;
    /// one that is left finishes its turn and rests label side out.
    pub fn set_focused(&mut self, focused: bool) {
        self.zoom_to = if focused { 1.0 } else { SHELF_REST };
        if focused {
            if !self.reduced {
                self.flips_left = SHOWCASE_FLIPS;
                self.hold = self.hold.min(0.15);
            }
        } else {
            // An odd number of half-turns so far leaves the read side out:
            // one more brings the label back round.
            let halves = (self.flip_to / std::f32::consts::PI).round() as i32;
            self.flips_left = u8::from(halves % 2 != 0);
            self.hold = self.hold.min(0.15);
        }
    }

    pub fn set_spinning(&mut self, on: bool) {
        self.spinning = on;
        if on {
            // Settle on whichever face is showing rather than whip around.
            self.flips_left = 0;
            self.flip_to = (self.flip_to / std::f32::consts::PI).round() * std::f32::consts::PI;
            self.flip_from = self.flip_to;
            self.flip_t = 1.0;
        }
    }

    fn showcasing(&self) -> bool {
        self.flips_left > 0 || self.flip_t < 1.0
    }

    pub fn step(&mut self, dt: f32) {
        if self.flip_t >= 1.0 {
            if self.flips_left > 0 {
                self.hold -= dt;
                if self.hold <= 0.0 {
                    self.flip_from = self.flip_to;
                    self.flip_to = self.flip_from + std::f32::consts::PI;
                    self.flip_t = 0.0;
                    self.flips_left -= 1;
                }
            }
        } else {
            self.flip_t = (self.flip_t + dt / FLIP_SECONDS).min(1.0);
            if self.flip_t >= 1.0 {
                self.hold = HOLD_SECONDS;
            }
        }

        // Sway keeps the disc alive while it is being shown off, and dies
        // away once it rests.
        self.sway_t += dt;
        let sway_amp = if self.showcasing() && !self.reduced {
            0.09
        } else {
            0.0
        };
        settle(&mut self.sway, sway_amp, 1.5, dt, 1e-4);
        settle(&mut self.yaw, 0.22, 2.6, dt, 1e-4);
        let tilt = if self.spinning { -0.34 } else { -0.16 };
        settle(&mut self.tilt, tilt, 3.2, dt, 1e-4);
        settle(&mut self.presence, 1.0, 4.0, dt, 2e-3);
        settle(&mut self.zoom, self.zoom_to, 9.0, dt, 1e-3);

        // Spin-up and spin-down are eased: a drive takes a moment to reach
        // speed, and it reads as weight.
        let rpm_target = if self.spinning { PLAYING_RPM } else { 0.0 };
        settle(
            &mut self.rpm,
            rpm_target,
            if self.spinning { 1.1 } else { 0.9 },
            dt,
            0.5,
        );
        let scale = if self.reduced { 0.25 } else { 1.0 };
        self.spin = (self.spin + self.rpm / 60.0 * std::f32::consts::TAU * dt * scale)
            % std::f32::consts::TAU;
    }

    /// Whether anything is still changing. When not, no frames are drawn.
    pub fn moving(&self) -> bool {
        self.showcasing()
            || self.rpm != 0.0
            || self.sway != 0.0
            || self.yaw != 0.22
            || self.tilt != if self.spinning { -0.34 } else { -0.16 }
            || self.presence != 1.0
            || self.zoom != self.zoom_to
    }

    pub fn pose(&self) -> Pose {
        let flip =
            self.flip_from + (self.flip_to - self.flip_from) * ease_in_out_cubic(self.flip_t);
        Pose {
            turn: flip + self.yaw + (self.sway_t * 0.55).sin() * self.sway,
            tilt: self.tilt,
            spin: self.spin,
            presence: self.presence,
            zoom: self.zoom,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(motion: &mut Motion, seconds: f32) {
        for _ in 0..(seconds * 60.0) as usize {
            motion.step(1.0 / 60.0);
        }
    }

    #[test]
    fn comes_to_rest_after_the_showcase() {
        let mut m = Motion::new(false);
        assert!(m.moving());
        run(&mut m, 15.0);
        assert!(!m.moving(), "still moving after the showcase");
    }

    #[test]
    fn a_shelved_disc_rests_label_out_after_losing_focus() {
        let mut m = Motion::shelved(false);
        m.set_focused(true);
        // Part way through the showcase: one half-turn in.
        run(&mut m, 1.6);
        m.set_focused(false);
        run(&mut m, 15.0);
        assert!(!m.moving(), "still moving after losing focus");
        let halves = (m.flip_to / std::f32::consts::PI).round() as i32;
        assert_eq!(halves % 2, 0, "rests read side out");
        assert_eq!(m.pose().zoom, SHELF_REST);
    }

    #[test]
    fn spins_while_playing_and_stops_after() {
        let mut m = Motion::new(false);
        m.set_spinning(true);
        run(&mut m, 10.0);
        assert!(m.moving());
        m.set_spinning(false);
        run(&mut m, 15.0);
        assert!(!m.moving(), "still moving after spin-down");
    }
}
