//! Incremental Slint model updates and slider drag protection (pure, unit-tested).
use slint::{Model, VecModel};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

/// How long worker values for a row are ignored after the last local slider change.
pub const HOLD: Duration = Duration::from_millis(1500);

/// Brings `model` to `new` with minimal mutations, matching rows by `key`.
/// Unchanged rows are untouched, so their widgets (e.g. a dragged slider) survive.
pub fn reconcile<T, K>(model: &VecModel<T>, new: Vec<T>, key: impl Fn(&T) -> K)
where
    T: Clone + PartialEq + 'static,
    K: PartialEq,
{
    let new_keys: Vec<K> = new.iter().map(&key).collect();
    for i in (0..model.row_count()).rev() {
        let k = key(&model.row_data(i).expect("row in range"));
        if !new_keys.contains(&k) {
            model.remove(i);
        }
    }
    for (i, row) in new.into_iter().enumerate() {
        if i < model.row_count() {
            let current = model.row_data(i).expect("row in range");
            if key(&current) == new_keys[i] {
                if current != row {
                    model.set_row_data(i, row);
                }
                continue;
            }
            // Same key further down: reorder by moving it up.
            let found = (i + 1..model.row_count())
                .find(|&j| key(&model.row_data(j).expect("row in range")) == new_keys[i]);
            if let Some(j) = found {
                model.remove(j);
            }
        }
        model.insert(i, row);
    }
    while model.row_count() > new_keys.len() {
        model.remove(model.row_count() - 1);
    }
}

/// Remembers which row ids the user changed locally and when.
#[derive(Default)]
pub struct LocalHold(HashMap<String, Instant>);

impl LocalHold {
    pub fn touch(&mut self, id: &str, now: Instant) {
        self.0.insert(id.to_owned(), now);
    }
    pub fn is_held(&mut self, id: &str, now: Instant) -> bool {
        self.0
            .retain(|_, t| now.saturating_duration_since(*t) < HOLD);
        self.0.contains_key(id)
    }
}

/// Replaces worker rows that are held by the model's current row (same key).
pub fn keep_held<T: Clone + 'static, K: AsRef<str>>(
    model: &VecModel<T>,
    rows: &mut [T],
    hold: &mut LocalHold,
    now: Instant,
    key: impl Fn(&T) -> K,
) {
    for row in rows {
        if hold.is_held(key(row).as_ref(), now)
            && let Some(old) = model.iter().find(|r| key(r).as_ref() == key(row).as_ref())
        {
            *row = old;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    type R = (&'static str, i32);
    fn ids(m: &VecModel<R>) -> Vec<R> {
        m.iter().collect()
    }
    fn run(from: Vec<R>, to: Vec<R>) -> Vec<R> {
        let m = VecModel::from(from);
        reconcile(&m, to, |r| r.0);
        ids(&m)
    }

    #[test]
    fn add_remove_reorder_update_noop() {
        assert_eq!(
            run(vec![("a", 1)], vec![("a", 1), ("b", 2)]),
            [("a", 1), ("b", 2)]
        );
        assert_eq!(run(vec![("a", 1), ("b", 2)], vec![("b", 2)]), [("b", 2)]);
        assert_eq!(
            run(
                vec![("a", 1), ("b", 2), ("c", 3)],
                vec![("c", 3), ("a", 1), ("b", 2)]
            ),
            [("c", 3), ("a", 1), ("b", 2)]
        );
        assert_eq!(
            run(vec![("a", 1), ("b", 2)], vec![("a", 9), ("b", 2)]),
            [("a", 9), ("b", 2)]
        );
        assert_eq!(run(vec![("a", 1)], vec![("a", 1)]), [("a", 1)]);
        assert_eq!(run(vec![("a", 1)], vec![]), []);
        assert_eq!(run(vec![], vec![("a", 1)]), [("a", 1)]);
    }

    #[test]
    fn hold_keeps_local_value_until_expiry() {
        let m = VecModel::from(vec![("a", 5), ("b", 2)]);
        let mut hold = LocalHold::default();
        let t0 = Instant::now();
        hold.touch("a", t0);
        let mut rows = vec![("a", 50), ("b", 3)];
        keep_held(
            &m,
            &mut rows,
            &mut hold,
            t0 + Duration::from_millis(500),
            |r| r.0,
        );
        assert_eq!(rows, [("a", 5), ("b", 3)]);
        let mut rows = vec![("a", 50), ("b", 3)];
        keep_held(&m, &mut rows, &mut hold, t0 + HOLD, |r| r.0);
        assert_eq!(rows, [("a", 50), ("b", 3)]);
    }
}
