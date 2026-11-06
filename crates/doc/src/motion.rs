//! Motion studies (format 29): keyframes that drive joints over time.
//!
//! A key says "joint `joint` has `value` at time `time`" (seconds). Between two keys of a
//! joint the value is interpolated linearly; before the first key it holds the first
//! value and after the last it holds the last one. Playing a study only sets joint values
//! for display (the engine's motion time); it never edits the joints themselves.

use crate::DocError;
use crate::component::Assembly;
use serde::{Deserialize, Serialize};

/// Most keys one design may hold.
pub const MAX_MOTION_KEYS: usize = 10_000;
/// Latest time a key may sit at, in seconds (a little over a day).
pub const MAX_MOTION_TIME: f64 = 1e5;
/// Two keys of one joint closer than this in time are the same key (seconds).
pub const KEY_TIME_TOLERANCE: f64 = 1e-9;

/// One keyframe: a joint's value at a time.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MotionKey {
    pub joint: u32,
    /// Seconds from the start of the study.
    pub time: f64,
    /// The joint's value then: radians for a turning joint, mm for a slider.
    pub value: f64,
}

/// The design's motion study: its keys, sorted by joint then time.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Motion {
    #[serde(default)]
    pub keys: Vec<MotionKey>,
}

fn bad(reason: impl Into<String>) -> DocError {
    DocError::BadFeature {
        reason: reason.into(),
    }
}

/// Fails unless `t` is a time a key may have.
pub fn check_time(t: f64) -> Result<(), DocError> {
    if t.is_finite() && (0.0..=MAX_MOTION_TIME).contains(&t) {
        Ok(())
    } else {
        Err(bad(format!(
            "a motion time must be a finite number of seconds from 0 to {MAX_MOTION_TIME}"
        )))
    }
}

impl Motion {
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    fn sort(&mut self) {
        self.keys
            .sort_by(|a, b| a.joint.cmp(&b.joint).then(a.time.total_cmp(&b.time)));
    }

    /// Adds a key, or replaces the joint's key at the same time. The value must be inside
    /// the joint's limits. Fails without changing anything.
    pub fn add_key(&mut self, asm: &Assembly, key: MotionKey) -> Result<(), DocError> {
        check_time(key.time)?;
        let joint = asm.joint(key.joint)?;
        joint.check_value(key.value)?;
        if let Some(k) = self
            .keys
            .iter_mut()
            .find(|k| k.joint == key.joint && (k.time - key.time).abs() <= KEY_TIME_TOLERANCE)
        {
            k.value = key.value;
            return Ok(());
        }
        if self.keys.len() >= MAX_MOTION_KEYS {
            return Err(bad(format!(
                "a design holds at most {MAX_MOTION_KEYS} motion keys"
            )));
        }
        self.keys.push(key);
        self.sort();
        Ok(())
    }

    /// Removes the joint's key at `time`.
    pub fn remove_key(&mut self, joint: u32, time: f64) -> Result<MotionKey, DocError> {
        let i = self
            .keys
            .iter()
            .position(|k| k.joint == joint && (k.time - time).abs() <= KEY_TIME_TOLERANCE)
            .ok_or_else(|| bad(format!("joint {joint} has no motion key at {time} s")))?;
        Ok(self.keys.remove(i))
    }

    /// Drops every key of a joint (when the joint is deleted).
    pub fn forget_joint(&mut self, joint: u32) {
        self.keys.retain(|k| k.joint != joint);
    }

    /// The joints that have keys, each once, in id order.
    pub fn joints(&self) -> Vec<u32> {
        let mut out: Vec<u32> = self.keys.iter().map(|k| k.joint).collect();
        out.dedup();
        out
    }

    /// The distinct key times, in order.
    pub fn times(&self) -> Vec<f64> {
        let mut out: Vec<f64> = self.keys.iter().map(|k| k.time).collect();
        out.sort_by(f64::total_cmp);
        out.dedup_by(|a, b| (*a - *b).abs() <= KEY_TIME_TOLERANCE);
        out
    }

    /// The last key's time, or 0 with no keys.
    pub fn duration(&self) -> f64 {
        self.keys.iter().map(|k| k.time).fold(0.0, f64::max)
    }

    /// A joint's value at time `t`, interpolated linearly between its keys, or `None`
    /// when the joint has no keys.
    pub fn value_at(&self, joint: u32, t: f64) -> Option<f64> {
        let keys: Vec<&MotionKey> = self.keys.iter().filter(|k| k.joint == joint).collect();
        let first = keys.first()?;
        if t <= first.time {
            return Some(first.value);
        }
        for w in keys.windows(2) {
            let (a, b) = (w[0], w[1]);
            if t <= b.time {
                let span = b.time - a.time;
                if span <= KEY_TIME_TOLERANCE {
                    return Some(b.value);
                }
                let s = (t - a.time) / span;
                return Some(a.value + (b.value - a.value) * s);
            }
        }
        keys.last().map(|k| k.value)
    }

    /// Checks keys read from a file against the assembly.
    pub fn check(&self, asm: &Assembly) -> Result<(), DocError> {
        if self.keys.len() > MAX_MOTION_KEYS {
            return Err(bad(format!(
                "a design holds at most {MAX_MOTION_KEYS} motion keys"
            )));
        }
        let mut seen: Vec<(u32, f64)> = Vec::new();
        for k in &self.keys {
            check_time(k.time)?;
            let joint = asm.joint(k.joint)?;
            if !k.value.is_finite() {
                return Err(bad("a motion key's value must be finite"));
            }
            joint.check_value(k.value)?;
            if seen
                .iter()
                .any(|(j, t)| *j == k.joint && (t - k.time).abs() <= KEY_TIME_TOLERANCE)
            {
                return Err(bad(format!(
                    "joint {} has two motion keys at {} s",
                    k.joint, k.time
                )));
            }
            seen.push((k.joint, k.time));
        }
        Ok(())
    }

    /// The keys as read from a file, sorted.
    pub fn sorted(mut self) -> Motion {
        self.sort();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn study(keys: &[(u32, f64, f64)]) -> Motion {
        Motion {
            keys: keys
                .iter()
                .map(|&(joint, time, value)| MotionKey { joint, time, value })
                .collect(),
        }
        .sorted()
    }

    #[test]
    fn values_are_linear_between_keys_and_held_outside_them() {
        // 0 at t = 0 and 90 at t = 2: a quarter of the way (t = 0.5) is 22.5.
        let m = study(&[(1, 2.0, 90.0), (1, 0.0, 0.0)]);
        assert_eq!(m.value_at(1, 0.5), Some(22.5));
        assert_eq!(m.value_at(1, -1.0), Some(0.0));
        assert_eq!(m.value_at(1, 5.0), Some(90.0));
        assert_eq!(m.value_at(2, 0.5), None);
        assert_eq!(m.times(), vec![0.0, 2.0]);
        assert_eq!(m.duration(), 2.0);
    }
}
