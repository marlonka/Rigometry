use crate::{
    hardware,
    model::*,
    storage::{
        self, ScanEvent, ScanHandle, ScanNode, ScanSummary,
        duplicates::{self, DuplicateEvent, DuplicateGroup, DuplicateHandle, DuplicateSummary},
    },
};
use crossbeam_channel::{Receiver, Sender, TryRecvError, bounded};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashSet, VecDeque},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const AGGREGATE_OVERFLOW: &str =
    "Aggregate exceeds the u64 byte/count range; totals are saturated lower bounds";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Page {
    #[default]
    Overview,
    Cpu,
    Gpu,
    Storage,
    Diagnostics,
    Settings,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub page: Page,
    pub theme: String,
    pub language: crate::i18n::Language,
    pub scale: f32,
    pub scale_revision: u8,
    pub path: String,
    pub reduce_motion: bool,
    /// Smallest file the duplicate search reads, in bytes.
    pub duplicate_min_size: u64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            page: Page::Overview,
            theme: "Dark".into(),
            language: crate::i18n::Language::System,
            scale: 1.0,
            scale_revision: 1,
            path: String::new(),
            reduce_motion: false,
            duplicate_min_size: duplicates::DEFAULT_MIN_SIZE,
        }
    }
}

enum HardwareEvent {
    Inventory(Inventory),
    Sample(Box<Telemetry>),
    Failed(String),
}
pub enum TaskResult {
    Folder {
        path: Option<PathBuf>,
        generation: u64,
    },
    Export(Result<PathBuf, String>),
    Cancelled,
    Failed(String),
}

pub struct State {
    pub settings: Settings,
    pub inventory: Option<Inventory>,
    pub history: VecDeque<Telemetry>,
    pub hardware_error: Option<String>,
    pub nodes: Arc<Vec<ScanNode>>,
    pub summary: Option<ScanSummary>,
    pub scan: Option<ScanHandle>,
    pub scan_started: Instant,
    pub scan_generation: u64,
    pub scope: usize,
    pub selected: usize,
    pub expanded: HashSet<usize>,
    pub filter: String,
    /// The filter the current `visible` rows were built from.
    pub visible_filter: String,
    pub search_error: Option<String>,
    /// The sort to restore when a search that switched to relevance ends.
    sort_before_search: Option<(Sort, bool)>,
    name_masks: crate::search::NameMasks,
    pub view: View,
    pub duplicates: Option<Duplicates>,
    pub sort: Sort,
    pub descending: bool,
    pub visible: Vec<(usize, usize)>,
    pub dirty: bool,
    /// A user action is waiting for the rows; skip the scan-time throttle.
    pub urgent: bool,
    pub last_index: Instant,
    pub map_ids: Vec<usize>,
    pub map_revision: u64,
    pub adapter: usize,
    pub notice: Option<String>,
    pub task: Option<Receiver<TaskResult>>,
    receiver: Receiver<HardwareEvent>,
    stop: Arc<AtomicBool>,
    /// Repaints the UI when a background scan or search queues results.
    wake: storage::Wake,
}
/// What the storage results show below the toolbar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    #[default]
    Hierarchy,
    Largest,
    Map,
    Duplicates,
}

/// One duplicate search of the current scan, and what it found so far.
pub struct Duplicates {
    pub handle: Option<DuplicateHandle>,
    /// The folder searched.
    pub root: usize,
    pub min_size: u64,
    pub groups: Vec<DuplicateGroup>,
    pub progress: Option<duplicates::Progress>,
    pub summary: Option<DuplicateSummary>,
    /// Groups to show, by index: those with a copy matching the search,
    /// largest savings first.
    pub visible: Vec<usize>,
    /// Table rows: each visible group followed by its copies.
    pub rows: Vec<DuplicateRow>,
    dirty: bool,
    last_index: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DuplicateRow {
    Group(usize),
    /// Group, then copy within it.
    Copy(usize, usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Name,
    /// Bytes on disk: what deleting an entry frees. Logical bytes are shown
    /// as detail only; for most files the two are identical.
    Allocated,
    Files,
    /// Search score, then name. Only active while searching.
    Relevance,
}

impl State {
    pub fn new(settings: Settings, ctx: eframe::egui::Context) -> Self {
        let (sender, receiver) = bounded(4);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let repaint = ctx.clone();
        let wake: storage::Wake = Arc::new(move || repaint.request_repaint());
        let hardware_error = std::thread::Builder::new()
            .name("hardware-monitor".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let inventory = hardware::discover();
                    let mut monitor = hardware::Monitor::new(&inventory);
                    if sender.send(HardwareEvent::Inventory(inventory)).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                    while !worker_stop.load(Ordering::Relaxed) {
                        let start = Instant::now();
                        let sample = monitor.sample();
                        if sender
                            .try_send(HardwareEvent::Sample(Box::new(sample)))
                            .is_err()
                            && worker_stop.load(Ordering::Relaxed)
                        {
                            break;
                        }
                        ctx.request_repaint();
                        let remaining = Duration::from_secs(1).saturating_sub(start.elapsed());
                        std::thread::sleep(remaining);
                    }
                }));
                if result.is_err() {
                    let _ = sender.try_send(HardwareEvent::Failed(
                        "Hardware worker stopped unexpectedly".into(),
                    ));
                    ctx.request_repaint();
                }
            })
            .err()
            .map(|error| format!("Could not start hardware worker: {error}"));
        Self::with_hardware(settings, receiver, stop, hardware_error, wake)
    }

    fn with_hardware(
        settings: Settings,
        receiver: Receiver<HardwareEvent>,
        stop: Arc<AtomicBool>,
        hardware_error: Option<String>,
        wake: storage::Wake,
    ) -> Self {
        Self {
            settings,
            inventory: None,
            history: VecDeque::with_capacity(121),
            hardware_error,
            nodes: Arc::new(vec![]),
            summary: None,
            scan: None,
            scan_started: Instant::now(),
            scan_generation: 0,
            scope: 0,
            selected: 0,
            expanded: HashSet::new(),
            filter: String::new(),
            visible_filter: String::new(),
            search_error: None,
            sort_before_search: None,
            name_masks: Default::default(),
            view: View::Hierarchy,
            duplicates: None,
            sort: Sort::Allocated,
            descending: true,
            visible: vec![],
            dirty: false,
            urgent: false,
            last_index: Instant::now(),
            map_ids: vec![],
            map_revision: 0,
            adapter: 0,
            notice: None,
            task: None,
            receiver,
            stop,
            wake,
        }
    }

    #[cfg(test)]
    pub(crate) fn fixture(settings: Settings) -> Self {
        // UI/state tests control hardware inputs; native providers are verified
        // separately. No driver thread can move a target between pointer events.
        Self::with_hardware(
            settings,
            crossbeam_channel::never(),
            Arc::new(AtomicBool::new(false)),
            None,
            Arc::new(|| {}),
        )
    }
    pub fn tick(&mut self) {
        loop {
            let event = match self.receiver.try_recv() {
                Ok(event) => event,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.hardware_error.get_or_insert_with(|| {
                        "Hardware worker stopped; readings may be stale".into()
                    });
                    break;
                }
            };
            match event {
                HardwareEvent::Inventory(value) => self.inventory = Some(value),
                HardwareEvent::Sample(value) => {
                    self.history.push_back(*value);
                    while self.history.len() > 120 {
                        self.history.pop_front();
                    }
                }
                HardwareEvent::Failed(value) => self.hardware_error = Some(value),
            }
        }
        let now = now_ms();
        while self
            .history
            .front()
            .is_some_and(|s| now.saturating_sub(s.timestamp_ms) > 120_000)
        {
            self.history.pop_front();
        }
        if let Some(s) = self.history.back_mut() {
            s.cpu_usage.mark_stale(now);
            s.memory_used.mark_stale(now);
            s.memory_total.mark_stale(now);
            s.cpu_frequency.mark_stale(now);
            for reading in &mut s.per_core_usage {
                reading.mark_stale(now);
            }
            for gpu in &mut s.gpus {
                for (_, r) in &mut gpu.readings {
                    r.mark_stale(now);
                }
            }
        }
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(8) {
            let event = self
                .scan
                .as_ref()
                .and_then(|scan| match scan.receiver.try_recv() {
                    Ok(event) => Some(event),
                    Err(TryRecvError::Empty) => None,
                    Err(TryRecvError::Disconnected) => Some(ScanEvent::Finished(ScanSummary {
                        cancelled: scan.cancel.load(Ordering::Relaxed),
                        stopped_early: true,
                        errors: 1,
                        incomplete_nodes: vec![0],
                        notes: vec![
                            "Scan worker stopped before completion; results are incomplete".into(),
                        ],
                        ..Default::default()
                    })),
                });
            match event {
                Some(ScanEvent::Batch(batch)) => {
                    append_batch(Arc::make_mut(&mut self.nodes), batch);
                    self.dirty = true;
                }
                Some(ScanEvent::Finished(mut summary)) => {
                    finalize_scan(Arc::make_mut(&mut self.nodes).as_mut_slice(), &mut summary);
                    self.summary = Some(summary);
                    self.scan = None;
                    self.dirty = true;
                    // Sizes stop changing: lay the map out once more.
                    self.map_revision = self.map_revision.wrapping_add(1);
                    break;
                }
                None => break,
            }
        }
        self.receive_duplicates(start);
        if self.dirty
            && (self.scan.is_none()
                || self.urgent
                || self.last_index.elapsed() > Duration::from_millis(400))
        {
            self.rebuild_visible();
            // From the end: a rebuild slower than the interval must not
            // run again on every frame of a scan.
            self.last_index = Instant::now();
        }
        if let Some(result) = self.task.as_ref().and_then(|r| match r.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(TaskResult::Failed(
                "Background task stopped before completion".into(),
            )),
        }) {
            self.task = None;
            match result {
                TaskResult::Folder {
                    path: Some(path),
                    generation,
                } => {
                    // A delayed folder dialog must not replace a scan the user
                    // started after opening that dialog.
                    if generation == self.scan_generation {
                        self.settings.path = path.to_string_lossy().into();
                        self.start_scan();
                    }
                }
                TaskResult::Folder { path: None, .. } | TaskResult::Cancelled => {}
                TaskResult::Failed(error) => self.notice = Some(error),
                TaskResult::Export(result) => {
                    self.notice = Some(match result {
                        Ok(path) => format!("Saved {}", path.display()),
                        Err(e) => format!("Export failed: {e}"),
                    })
                }
            }
        }
    }
    pub fn start_scan(&mut self) {
        let path = storage::scan_input_path(&self.settings.path);
        if path.len() != self.settings.path.len() {
            self.settings.path = path.to_owned();
        }
        if self.settings.path.is_empty() {
            self.notice = Some("Enter a folder or drive path".into());
            return;
        }
        self.cancel_scan();
        // Results name entries of the old tree; dropping them stops the search.
        self.duplicates = None;
        self.nodes = Arc::new(vec![]);
        self.visible.clear();
        self.summary = None;
        self.scope = 0;
        self.selected = 0;
        self.expanded.clear();
        self.expanded.insert(0);
        self.map_ids.clear();
        self.map_revision = self.map_revision.wrapping_add(1);
        self.scan_generation += 1;
        self.scan_started = Instant::now();
        self.notice = None;
        self.dirty = true;
        self.scan = Some(storage::start_scan_with(
            PathBuf::from(&self.settings.path),
            self.wake.clone(),
        ));
    }
    pub fn cancel_scan(&mut self) {
        if let Some(scan) = &self.scan {
            scan.cancel.store(true, Ordering::Relaxed);
        }
    }
    pub fn pick_folder(&mut self) {
        if self.task.is_some() {
            return;
        }
        let generation = self.scan_generation;
        let language = self.settings.language.resolve();
        self.spawn_task("folder-dialog", move |tx| {
            let _ = tx.send(TaskResult::Folder {
                path: rfd::FileDialog::new()
                    .set_title(language.text("Scan folder"))
                    .pick_folder(),
                generation,
            });
        });
    }
    fn spawn_task(&mut self, name: &str, run: impl FnOnce(Sender<TaskResult>) + Send + 'static) {
        let (tx, rx) = bounded(1);
        match std::thread::Builder::new()
            .name(name.into())
            .spawn(move || run(tx))
        {
            Ok(_) => self.task = Some(rx),
            Err(error) => self.notice = Some(format!("Could not start background task: {error}")),
        }
    }
    pub fn export(&mut self, format: &str) {
        if self.task.is_some() || self.scan.is_some() || self.nodes.is_empty() {
            return;
        }
        let nodes = self.nodes.clone();
        let summary = self.summary.clone();
        let format = format.to_owned();
        let language = self.settings.language.resolve();
        self.spawn_task("scan-export", move |tx| {
            if let Some(path) = rfd::FileDialog::new()
                .set_title(language.text("Export scan"))
                .add_filter(format.to_uppercase(), &[&format])
                .set_file_name(format!("rigometry-scan.{format}"))
                .save_file()
            {
                let result = if format == "csv" {
                    storage::export_csv(&path, &nodes, summary.as_ref())
                } else {
                    storage::export_json(&path, &nodes, summary.as_ref())
                };
                let _ = tx.send(TaskResult::Export(result.map(|_| path)));
            } else {
                let _ = tx.send(TaskResult::Cancelled);
            }
        });
    }
    /// Compares the files below the current folder. Needs a finished scan:
    /// the search reads the tree while the UI keeps showing it.
    pub fn find_duplicates(&mut self) {
        if self.scan.is_some() || self.nodes.is_empty() {
            return;
        }
        let min_size = self.settings.duplicate_min_size.max(1);
        self.duplicates = Some(Duplicates {
            handle: Some(duplicates::find(
                self.nodes.clone(),
                self.scope,
                min_size,
                self.summary.as_ref().is_none_or(|summary| {
                    storage::subtree_partial(&self.nodes, summary, self.scope)
                }),
                self.wake.clone(),
            )),
            root: self.scope,
            min_size,
            groups: vec![],
            progress: None,
            summary: None,
            visible: vec![],
            rows: vec![],
            dirty: false,
            last_index: Instant::now(),
        });
    }

    pub fn duplicates_running(&self) -> bool {
        self.duplicates.as_ref().is_some_and(|d| d.handle.is_some())
    }

    pub fn cancel_duplicates(&mut self) {
        if let Some(handle) = self.duplicates.as_ref().and_then(|d| d.handle.as_ref()) {
            handle.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Takes the duplicate search's events for the rest of this frame's budget.
    fn receive_duplicates(&mut self, start: Instant) {
        let Some(found) = &mut self.duplicates else {
            return;
        };
        while let Some(handle) = &found.handle {
            let event = match handle.receiver.try_recv() {
                Ok(event) => event,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => DuplicateEvent::Finished(DuplicateSummary {
                    root: found.root,
                    min_size: found.min_size,
                    cancelled: handle.cancel.load(Ordering::Relaxed),
                    errors: 1,
                    notes: vec![
                        "Duplicate search stopped before completion; results are incomplete".into(),
                    ],
                    ..Default::default()
                }),
            };
            match event {
                DuplicateEvent::Progress(progress) => found.progress = Some(progress),
                DuplicateEvent::Groups(groups) => {
                    found.groups.extend(groups);
                    found.dirty = true;
                }
                DuplicateEvent::Finished(summary) => {
                    found.summary = Some(summary);
                    found.handle = None;
                    found.dirty = true;
                }
            }
            if start.elapsed() > Duration::from_millis(8) {
                break;
            }
        }
        if found.dirty
            && (found.handle.is_none()
                || self.urgent
                || found.last_index.elapsed() > Duration::from_millis(400))
        {
            self.rebuild_duplicates();
            if let Some(found) = &mut self.duplicates {
                found.last_index = Instant::now();
            }
        }
    }

    /// Filters the groups by the search and orders them by savings.
    pub fn rebuild_duplicates(&mut self) {
        let Some(found) = &mut self.duplicates else {
            return;
        };
        found.dirty = false;
        let groups = &found.groups;
        // An invalid search shows its error and no groups, as the table does.
        let Ok(query) = crate::search::Query::parse(&self.filter) else {
            found.visible.clear();
            found.rows.clear();
            return;
        };
        let mut visible: Vec<usize> = if query.is_empty() {
            (0..groups.len()).collect()
        } else {
            let masks = self.name_masks.update(&self.nodes, self.scan_generation);
            let mut matches = vec![false; self.nodes.len()];
            for hit in query.search(&self.nodes, masks, found.root) {
                matches[hit.id] = true;
            }
            (0..groups.len())
                .filter(|&g| groups[g].copies.iter().any(|copy| matches[copy.id]))
                .collect()
        };
        visible.sort_by_key(|&g| groups[g].order());
        found.rows.clear();
        for &g in &visible {
            found.rows.push(DuplicateRow::Group(g));
            found
                .rows
                .extend((0..groups[g].copies.len()).map(|c| DuplicateRow::Copy(g, c)));
        }
        found.visible = visible;
    }

    pub fn export_duplicates(&mut self, format: &str) {
        let Some(found) = &self.duplicates else {
            return;
        };
        let Some(summary) = found.summary.clone() else {
            return;
        };
        if self.task.is_some() {
            return;
        }
        let nodes = self.nodes.clone();
        let groups: Vec<_> = found
            .visible
            .iter()
            .map(|&g| found.groups[g].clone())
            .collect();
        let format = format.to_owned();
        let language = self.settings.language.resolve();
        self.spawn_task("duplicates-export", move |tx| {
            if let Some(path) = rfd::FileDialog::new()
                .set_title(language.text("Export duplicates"))
                .add_filter(format.to_uppercase(), &[&format])
                .set_file_name(format!("rigometry-duplicates.{format}"))
                .save_file()
            {
                let result = if format == "csv" {
                    duplicates::export_csv(&path, &nodes, &groups, &summary)
                } else {
                    duplicates::export_json(&path, &nodes, &groups, &summary)
                };
                let _ = tx.send(TaskResult::Export(result.map(|_| path)));
            } else {
                let _ = tx.send(TaskResult::Cancelled);
            }
        });
    }

    pub fn export_hardware(&mut self) {
        if self.task.is_some() {
            return;
        }
        let Some(inv) = self.inventory.clone() else {
            return;
        };
        let samples: Vec<_> = self.history.iter().cloned().collect();
        let language = self.settings.language.resolve();
        self.spawn_task("hardware-export", move |tx| {
            if let Some(path) = rfd::FileDialog::new()
                .set_title(language.text("Export hardware"))
                .add_filter("JSON", &["json"])
                .set_file_name("rigometry-hardware.json")
                .save_file()
            {
                let result = serde_json::to_vec_pretty(
                    &serde_json::json!({"schema_version":1,"inventory":inv,"samples":samples}),
                )
                .map_err(|e| e.to_string())
                .and_then(|data| storage::write_new_output(&path, &data));
                let _ = tx.send(TaskResult::Export(result.map(|_| path)));
            } else {
                let _ = tx.send(TaskResult::Cancelled);
            }
        });
    }
    pub fn navigate_scope(&mut self, id: usize) {
        self.scope = id;
        self.selected = id;
        self.expanded.insert(id);
        self.refresh_now();
    }
    /// Searching ranks by relevance until a column is chosen; clearing the
    /// search restores the sort that was active before it.
    pub fn filter_changed(&mut self) {
        // A lone `!` or `^` searches for nothing yet and keeps the sort.
        let searching = !crate::search::Query::parse(&self.filter).is_ok_and(|q| q.is_empty());
        if searching && self.sort_before_search.is_none() {
            self.sort_before_search = Some((self.sort, self.descending));
            self.sort = Sort::Relevance;
            self.descending = true;
        } else if !searching
            && let Some((sort, descending)) = self.sort_before_search.take()
            && self.sort == Sort::Relevance
        {
            (self.sort, self.descending) = (sort, descending);
        }
        self.refresh_now();
    }
    /// Rebuild rows on the next frame, even while a scan is streaming in.
    pub fn refresh_now(&mut self) {
        self.dirty = true;
        self.urgent = true;
        if let Some(found) = &mut self.duplicates {
            found.dirty = true;
        }
    }
    pub fn rebuild_visible(&mut self) {
        self.visible.clear();
        self.dirty = false;
        self.urgent = false;
        self.visible_filter.clone_from(&self.filter);
        if self.nodes.is_empty() {
            self.map_ids.clear();
            self.map_revision = self.map_revision.wrapping_add(1);
            return;
        }
        let mut map_ids: Vec<usize> = self.nodes[self.scope]
            .children
            .iter()
            .copied()
            .filter(|id| self.nodes[*id].allocated > 0)
            .collect();
        map_ids.sort_unstable_by_key(|id| std::cmp::Reverse(self.nodes[*id].allocated));
        // Typing, sorting and expanding keep the map's layout; only new
        // tiles or sizes lay it out again.
        if map_ids != self.map_ids || self.scan.is_some() {
            self.map_ids = map_ids;
            self.map_revision = self.map_revision.wrapping_add(1);
        }
        let query = match crate::search::Query::parse(&self.filter) {
            Ok(query) => {
                self.search_error = None;
                query
            }
            Err(error) => {
                self.search_error = Some(error);
                return;
            }
        };
        let compare = |a: &usize, b: &usize| {
            let a = &self.nodes[*a];
            let b = &self.nodes[*b];
            let order = match self.sort {
                Sort::Name => name_order(&a.name, &b.name),
                Sort::Allocated | Sort::Relevance => a.allocated.cmp(&b.allocated),
                Sort::Files => a.files.cmp(&b.files),
            };
            let order = if self.descending {
                order.reverse()
            } else {
                order
            };
            order.then_with(|| a.name.cmp(&b.name))
        };
        let largest = self.view == View::Largest;
        if largest || !query.is_empty() {
            let masks = self.name_masks.update(&self.nodes, self.scan_generation);
            let mut hits = query.search(&self.nodes, masks, self.scope);
            if largest {
                hits.retain(|hit| !self.nodes[hit.id].is_dir);
            }
            if self.sort == Sort::Relevance {
                // Ties go by name, not size: sizes grow while a scan runs, and
                // rows must not reshuffle under the pointer.
                hits.sort_by(|a, b| {
                    b.score
                        .cmp(&a.score)
                        .then_with(|| name_order(&self.nodes[a.id].name, &self.nodes[b.id].name))
                });
            } else {
                hits.sort_by(|a, b| compare(&a.id, &b.id));
            }
            self.visible.extend(hits.into_iter().map(|hit| (hit.id, 0)));
        } else {
            let mut roots = self.nodes[self.scope].children.clone();
            roots.sort_by(compare);
            let mut stack: Vec<_> = roots.into_iter().rev().map(|id| (id, 0)).collect();
            while let Some((id, depth)) = stack.pop() {
                self.visible.push((id, depth));
                if self.expanded.contains(&id) {
                    let mut children = self.nodes[id].children.clone();
                    children.sort_by(compare);
                    stack.extend(children.into_iter().rev().map(|n| (n, depth + 1)));
                }
            }
        }
    }
}

/// Case-insensitive name order without allocating: sorting millions of
/// entries by name must not build two lowercase strings per comparison.
fn name_order(a: &str, b: &str) -> std::cmp::Ordering {
    if a.is_ascii() && b.is_ascii() {
        a.bytes()
            .map(|c| c.to_ascii_lowercase())
            .cmp(b.bytes().map(|c| c.to_ascii_lowercase()))
    } else {
        a.chars()
            .flat_map(char::to_lowercase)
            .cmp(b.chars().flat_map(char::to_lowercase))
    }
}

pub fn append_batch(nodes: &mut Vec<ScanNode>, batch: Vec<ScanNode>) {
    for mut node in batch {
        // A scan owns one receiver, so late batches from replaced scans cannot
        // enter this tree; a parent always precedes its children.
        if node.id != nodes.len() || node.parent.is_some_and(|p| p >= node.id) {
            continue;
        }
        let (logical, allocated, files, incomplete) =
            (node.logical, node.allocated, node.files, node.incomplete);
        let mut parent = node.parent;
        node.children.clear();
        if let Some(p) = parent {
            nodes[p].children.push(node.id);
        }
        nodes.push(node);
        while let Some(p) = parent {
            let node = &mut nodes[p];
            let logical = node.logical.checked_add(logical);
            let allocated = node.allocated.checked_add(allocated);
            let files = node.files.checked_add(files);
            let overflow = logical.is_none() || allocated.is_none() || files.is_none();
            node.logical = logical.unwrap_or(u64::MAX);
            node.allocated = allocated.unwrap_or(u64::MAX);
            node.files = files.unwrap_or(u64::MAX);
            node.incomplete |= incomplete || overflow;
            if overflow && !node.note.contains(AGGREGATE_OVERFLOW) {
                if !node.note.is_empty() {
                    node.note.push('\n');
                }
                node.note.push_str(AGGREGATE_OVERFLOW);
            }
            parent = node.parent;
        }
    }
}
pub fn finalize_scan(nodes: &mut [ScanNode], summary: &mut ScanSummary) {
    // The root's totals include every descendant's, so any overflow reaches it.
    if nodes
        .first()
        .is_some_and(|root| root.note.contains(AGGREGATE_OVERFLOW))
        && !summary.notes.iter().any(|note| note == AGGREGATE_OVERFLOW)
    {
        summary.errors = summary.errors.saturating_add(1);
        summary.notes.push(AGGREGATE_OVERFLOW.into());
        if !summary.incomplete_nodes.contains(&0) {
            summary.incomplete_nodes.push(0);
        }
    }
    if summary.cancelled || summary.stopped_early {
        for node in nodes.iter_mut().filter(|n| n.is_dir) {
            node.incomplete = true;
        }
    }
    for &id in &summary.incomplete_nodes {
        let mut next = Some(id);
        while let Some(id) = next {
            let Some(node) = nodes.get_mut(id) else {
                break;
            };
            node.incomplete = true;
            next = node.parent;
        }
    }
}
impl Drop for State {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.cancel_scan();
        self.cancel_duplicates();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disconnected_workers_expose_partial_scan_and_release_pending_task() {
        let mut state = State::fixture(Settings::default());
        state.stop.store(true, Ordering::Relaxed);
        let (sender, receiver) = bounded(1);
        state.scan = Some(ScanHandle {
            receiver,
            cancel: Arc::new(AtomicBool::new(false)),
        });
        state.nodes = Arc::new(vec![ScanNode {
            id: 0,
            parent: None,
            name: "fixture".into(),
            is_dir: true,
            logical: 123,
            allocated: 128,
            files: 1,
            children: vec![],
            note: String::new(),
            incomplete: false,
        }]);
        drop(sender);
        state.tick();
        assert!(
            state.scan.is_none(),
            "a disconnected scan must not leave the UI scanning forever"
        );
        assert_eq!(state.summary.as_ref().unwrap().errors, 1);
        assert!(state.nodes[0].incomplete);
        assert_eq!(state.nodes[0].logical, 123, "keep known partial totals");
        let (sender, receiver) = bounded(1);
        state.task = Some(receiver);
        drop(sender);
        state.tick();
        assert!(
            state.task.is_none(),
            "a failed dialog/export must not disable future actions"
        );
        assert!(
            state
                .notice
                .as_ref()
                .unwrap()
                .contains("stopped before completion")
        );
    }

    #[test]
    fn delayed_folder_dialog_cannot_replace_a_newer_scan() {
        let fixture = tempfile::tempdir().unwrap();
        let mut state = State::fixture(Settings::default());
        state.stop.store(true, Ordering::Relaxed);
        state.scan_generation = 2;
        state.settings.path = "newer selection".into();
        let (sender, receiver) = bounded(1);
        sender
            .send(TaskResult::Folder {
                path: Some(fixture.path().into()),
                generation: 1,
            })
            .unwrap();
        state.task = Some(receiver);
        state.tick();
        assert_eq!(state.settings.path, "newer selection");
        assert_eq!(state.scan_generation, 2);
        let (sender, receiver) = bounded(1);
        sender
            .send(TaskResult::Folder {
                path: Some(fixture.path().into()),
                generation: 2,
            })
            .unwrap();
        state.task = Some(receiver);
        state.tick();
        assert_eq!(state.settings.path, fixture.path().to_string_lossy());
        assert_eq!(
            state.scan_generation, 3,
            "the current dialog still starts its selected scan"
        );
    }
    #[test]
    fn aggregate_directory_streams_and_files_once() {
        let node = |id, parent, dir, logical| ScanNode {
            id,
            parent,
            name: id.to_string(),
            is_dir: dir,
            logical,
            allocated: logical,
            files: if dir { 0 } else { 1 },
            children: vec![],
            note: String::new(),
            incomplete: false,
        };
        let mut nodes = vec![];
        append_batch(
            &mut nodes,
            vec![node(0, None, true, 2), node(1, Some(0), true, 3)],
        );
        append_batch(&mut nodes, vec![node(2, Some(1), false, 7)]);
        assert_eq!(
            (nodes[0].logical, nodes[1].logical, nodes[2].logical),
            (12, 10, 7)
        );
        assert_eq!(nodes[0].files, 1);
        assert_eq!(nodes[0].children, vec![1]);
    }

    #[test]
    fn aggregate_overflow_remains_partial_in_exported_accounting() {
        let node = |id, logical, allocated, files| ScanNode {
            id,
            parent: (id != 0).then_some(0),
            name: id.to_string(),
            is_dir: id == 0,
            logical,
            allocated,
            files,
            children: vec![],
            note: String::new(),
            incomplete: false,
        };
        // Exercise each independently: logical sparse data, allocated streams,
        // and directory file counts must never wrap or appear exact on overflow.
        for overflow_field in 0..3 {
            let mut values = [0, 0, 0];
            values[overflow_field] = u64::MAX;
            let mut nodes = vec![];
            append_batch(
                &mut nodes,
                vec![node(0, 0, 0, 0), node(1, values[0], values[1], values[2])],
            );
            assert!(!nodes[0].incomplete, "u64::MAX itself is representable");
            let mut additional = [0, 0, 0];
            additional[overflow_field] = 1;
            append_batch(
                &mut nodes,
                vec![node(2, additional[0], additional[1], additional[2])],
            );
            assert!(
                nodes[0].incomplete,
                "overflow must mark the total as a lower bound"
            );
            assert_eq!(
                [nodes[0].logical, nodes[0].allocated, nodes[0].files][overflow_field],
                u64::MAX
            );
            let mut summary = ScanSummary::default();
            finalize_scan(&mut nodes, &mut summary);
            finalize_scan(&mut nodes, &mut summary);
            assert_eq!(
                summary.errors, 1,
                "finalization must not count overflow twice"
            );
            assert_eq!(summary.incomplete_nodes, [0]);
            assert!(
                summary
                    .notes
                    .iter()
                    .any(|note| note.contains("saturated lower bounds"))
            );
            let fixture = tempfile::tempdir().unwrap();
            let path = fixture.path().join("overflow.json");
            storage::export_json(&path, &nodes, Some(&summary)).unwrap();
            let report: serde_json::Value =
                serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            assert_eq!(report["status"], "partial");
            assert_eq!(report["nodes"][0]["incomplete"], true);
        }
    }

    #[test]
    fn stopped_scan_cannot_claim_unvisited_folders_are_empty() {
        let node = |id, parent, is_dir, logical| ScanNode {
            id,
            parent,
            is_dir,
            logical,
            allocated: logical,
            files: if is_dir { 0 } else { 1 },
            name: id.to_string(),
            children: vec![],
            note: String::new(),
            incomplete: false,
        };
        let mut nodes = vec![];
        append_batch(
            &mut nodes,
            vec![
                node(0, None, true, 0),
                node(1, Some(0), true, 0),
                node(2, Some(0), false, 42),
            ],
        );
        let mut summary = ScanSummary {
            stopped_early: true,
            incomplete_nodes: vec![0],
            ..Default::default()
        };
        finalize_scan(&mut nodes, &mut summary);
        assert!(nodes[0].incomplete);
        assert!(
            nodes[1].incomplete,
            "a discovered but unvisited folder has an unknown total"
        );
        assert_eq!(nodes[1].logical, 0, "zero remains only a known lower bound");
        assert!(
            !nodes[2].incomplete,
            "a fully read file keeps its measured size"
        );
        assert_eq!(nodes[2].logical, 42);
    }
}
