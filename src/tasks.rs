//! A daily checklist. Unfinished tasks carry over; tasks completed on an earlier day are cleared
//! at the first moment of a new day.

use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};

use serde::{Deserialize, Serialize};
use slint::{Model, VecModel};

use crate::{TaskData, storage};

const FILE: &str = "tasks.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub title: String,
    pub done: bool,
    /// Local calendar day ("2026-11-14") the task was completed on.
    #[serde(default)]
    pub completed_on: Option<String>,
}

pub struct TaskList {
    items: Vec<Task>,
    model: Rc<VecModel<TaskData>>,
}

fn new_id() -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{nanos:x}-{:x}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

fn row(t: &Task) -> TaskData {
    TaskData { id: t.id.as_str().into(), title: t.title.as_str().into(), done: t.done }
}

impl TaskList {
    pub fn load() -> Self {
        // A damaged entry costs only that task, not the list.
        let mut items: Vec<Task> = storage::load_json_salvaged(FILE).unwrap_or_default();
        items.retain(|t| !t.title.trim().is_empty());
        items.sort_by_key(|t| t.done); // stable: open tasks first, order otherwise preserved
        Self::from_items(items)
    }

    pub fn from_items(items: Vec<Task>) -> Self {
        let model = Rc::new(VecModel::from(items.iter().map(row).collect::<Vec<_>>()));
        Self { items, model }
    }

    pub fn model(&self) -> Rc<VecModel<TaskData>> {
        self.model.clone()
    }

    #[cfg(test)]
    pub fn items(&self) -> &[Task] {
        &self.items
    }

    pub fn total(&self) -> usize {
        self.items.len()
    }

    pub fn done(&self) -> usize {
        self.items.iter().filter(|t| t.done).count()
    }

    pub fn summary(&self) -> String {
        match (self.total(), self.done()) {
            (0, _) => "Nothing planned yet".into(),
            (t, d) if t == d => "All done".into(),
            (t, d) => format!("{d} of {t} done"),
        }
    }

    fn first_done(&self) -> Option<usize> {
        self.items.iter().position(|t| t.done)
    }

    /// Adds a task above any completed ones. Returns false for blank input.
    pub fn add(&mut self, title: &str) -> bool {
        let title = title.trim();
        if title.is_empty() {
            return false;
        }
        let task = Task { id: new_id(), title: title.chars().take(200).collect(), done: false, completed_on: None };
        let at = self.first_done().unwrap_or(self.items.len());
        self.model.insert(at, row(&task));
        self.items.insert(at, task);
        true
    }

    /// Toggles a task; returns its new state.
    pub fn toggle(&mut self, id: &str, today: &str) -> Option<bool> {
        let i = self.items.iter().position(|t| t.id == id)?;
        let t = &mut self.items[i];
        t.done = !t.done;
        t.completed_on = t.done.then(|| today.to_string());
        let done = t.done;
        self.model.set_row_data(i, row(t));
        if !done {
            // Reopened: move back above the completed block right away.
            if let Some(first) = self.items.iter().enumerate().position(|(j, t)| t.done && j != i)
                && first < i
            {
                self.move_item(i, first);
            }
        }
        Some(done)
    }

    /// After the completion animation, settle a finished task below the open ones.
    pub fn settle(&mut self, id: &str) {
        if let Some(i) = self.items.iter().position(|t| t.id == id && t.done) {
            let last = self.items.len() - 1;
            if i != last {
                self.move_item(i, last);
            }
        }
    }

    fn move_item(&mut self, from: usize, to: usize) {
        let t = self.items.remove(from);
        self.model.remove(from);
        self.model.insert(to, row(&t));
        self.items.insert(to, t);
    }

    pub fn remove(&mut self, id: &str) {
        if let Some(i) = self.items.iter().position(|t| t.id == id) {
            self.items.remove(i);
            self.model.remove(i);
        }
    }

    pub fn clear_completed(&mut self) {
        self.retain(|t| !t.done);
    }

    /// Drops tasks completed before `today`. Returns true if anything changed.
    pub fn rollover(&mut self, today: &str) -> bool {
        let before = self.items.len();
        self.retain(|t| !(t.done && t.completed_on.as_deref().is_none_or(|d| d < today)));
        before != self.items.len()
    }

    fn retain(&mut self, keep: impl Fn(&Task) -> bool) {
        self.items.retain(|t| keep(t));
        self.model.set_vec(self.items.iter().map(row).collect::<Vec<_>>());
    }

    pub fn save(&self) {
        storage::save_json(FILE, &self.items);
    }

    #[cfg(test)]
    fn titles(&self) -> Vec<&str> {
        self.items.iter().map(|t| t.title.as_str()).collect()
    }

    #[cfg(test)]
    fn model_matches(&self) -> bool {
        self.model.row_count() == self.items.len()
            && self.items.iter().enumerate().all(|(i, t)| {
                self.model.row_data(i).map(|r| r.id == t.id.as_str() && r.done == t.done).unwrap_or(false)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_damaged_task_costs_only_that_task() {
        let file = serde_json::json!([
            { "id": "a", "title": "Groceries", "done": false },
            { "id": "b", "title": 42, "done": "maybe" },
            { "id": "c", "title": "Call mum", "done": true, "completedOn": "2026-11-14" },
        ]);
        let (value, dropped) = crate::storage::salvage::<Vec<super::Task>>(file);
        let tasks: Vec<super::Task> = serde_json::from_value(value).unwrap();
        assert_eq!(dropped, ["[1]"]);
        let titles: Vec<&str> = tasks.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, ["Groceries", "Call mum"]);
    }

    fn list(titles: &[&str]) -> TaskList {
        let mut l = TaskList::from_items(Vec::new());
        for t in titles {
            l.add(t);
        }
        l
    }

    #[test]
    fn add_ignores_blank_and_inserts_above_completed() {
        let mut l = list(&["a", "b"]);
        assert!(!l.add("   "));
        let id = l.items()[0].id.clone();
        l.toggle(&id, "2026-11-14");
        l.settle(&id);
        l.add("c");
        assert_eq!(l.titles(), ["b", "c", "a"]);
        assert!(l.model_matches());
    }

    #[test]
    fn reopening_moves_task_above_completed_block() {
        let mut l = list(&["a", "b", "c"]);
        let (a, b) = (l.items()[0].id.clone(), l.items()[1].id.clone());
        l.toggle(&a, "d");
        l.settle(&a);
        l.toggle(&b, "d");
        l.settle(&b);
        assert_eq!(l.titles(), ["c", "a", "b"]);
        l.toggle(&b, "d");
        assert_eq!(l.titles(), ["c", "b", "a"]);
        assert!(l.model_matches());
    }

    #[test]
    fn rollover_clears_only_previous_days() {
        let mut l = list(&["old", "today", "open"]);
        let (old, today) = (l.items()[0].id.clone(), l.items()[1].id.clone());
        l.toggle(&old, "2026-11-13");
        l.toggle(&today, "2026-11-14");
        assert!(l.rollover("2026-11-14"));
        assert_eq!(l.titles(), ["today", "open"]);
        assert!(!l.rollover("2026-11-14"));
        assert!(l.model_matches());
    }

    #[test]
    fn summary_text() {
        let mut l = list(&["a"]);
        assert_eq!(l.summary(), "0 of 1 done");
        let id = l.items()[0].id.clone();
        l.toggle(&id, "d");
        assert_eq!(l.summary(), "All done");
    }
}
