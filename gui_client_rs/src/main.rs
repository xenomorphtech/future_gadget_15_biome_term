use std::{
    collections::{HashMap, VecDeque},
    fs,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use biome_term_client::{
    BiomeTermClient, BiomeTermClientBuilder, CreatePaneOptions, LifecycleEvent, PaneInfo,
};
use eframe::egui;
use futures_util::StreamExt;

const BLACK: egui::Color32 = egui::Color32::BLACK;
const GREEN: egui::Color32 = egui::Color32::from_rgb(0, 255, 0);
const DIM_GREEN: egui::Color32 = egui::Color32::from_rgb(0, 160, 0);
const DARK_GREEN: egui::Color32 = egui::Color32::from_rgb(0, 32, 0);

// ── Messages from background tasks ───────────────────────────────────────────

enum Msg {
    PanesUpdated(Vec<PaneInfo>),
    PaneOutput {
        pane_id: String,
        bytes: usize,
        received_at: Instant,
    },
    Error(String),
}

// ── Per-pane state ────────────────────────────────────────────────────────────

struct PaneState {
    parser: Arc<Mutex<vt100::Parser>>,
    task: tokio::task::JoinHandle<()>,
    throughput: ThroughputWindow,
}

#[derive(Clone)]
struct ConnectionSettings {
    url: String,
    api_key: String,
    ca_cert_path: String,
    insecure_tls: bool,
}

#[derive(Default)]
struct ThroughputWindow {
    samples: VecDeque<(Instant, usize)>,
    total_bytes: usize,
}

impl ThroughputWindow {
    const WINDOW: Duration = Duration::from_secs(5);

    fn record(&mut self, now: Instant, bytes: usize) {
        if bytes == 0 {
            self.prune(now);
            return;
        }

        self.samples.push_back((now, bytes));
        self.total_bytes += bytes;
        self.prune(now);
    }

    fn bytes_per_second(&mut self, now: Instant) -> f64 {
        self.prune(now);
        self.total_bytes as f64 / Self::WINDOW.as_secs_f64()
    }

    fn prune(&mut self, now: Instant) {
        while let Some((sample_at, sample_bytes)) = self.samples.front().copied() {
            if now.duration_since(sample_at) <= Self::WINDOW {
                break;
            }
            self.samples.pop_front();
            self.total_bytes = self.total_bytes.saturating_sub(sample_bytes);
        }
    }
}

// ── App ───────────────────────────────────────────────────────────────────────

struct App {
    rt: tokio::runtime::Runtime,
    client: Arc<BiomeTermClient>,
    tx: std::sync::mpsc::SyncSender<Msg>,
    rx: std::sync::mpsc::Receiver<Msg>,
    lifecycle_task: Option<tokio::task::JoinHandle<()>>,

    panes: Vec<PaneInfo>,
    pane_states: HashMap<String, PaneState>,
    selected_id: Option<String>,

    input: String,
    new_pane_name: String,
    server_url: String,
    url_buf: String,
    api_key_buf: String,
    ca_cert_path_buf: String,
    insecure_tls_buf: bool,
    font_size: f32,
    status: String,

    /// Terminal area has keyboard focus — all keys go straight to the PTY.
    terminal_focused: bool,
    /// Last rendered terminal panel rect, used to release direct input mode on outside clicks.
    terminal_rect: Option<egui::Rect>,
    /// Resize dimensions to send on demand (not auto-applied).
    resize_cols: u16,
    resize_rows: u16,

    /// Global input history (all panes share one history, like a shell).
    history: Vec<String>,
    /// Current position while browsing history; `None` = live input.
    history_pos: Option<usize>,
    /// Saved live input so Down-arrow restores it after browsing.
    history_draft: String,
}

impl App {
    fn new(cc: &eframe::CreationContext, settings: ConnectionSettings) -> Self {
        apply_theme(&cc.egui_ctx);

        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let (tx, rx) = std::sync::mpsc::sync_channel(512);
        let (client, server_url, status) = match build_client(&settings) {
            Ok(client) => (Arc::new(client), settings.url.clone(), String::new()),
            Err(err) => {
                let fallback_url = "http://localhost:3021".to_string();
                (
                    Arc::new(BiomeTermClient::new(&fallback_url)),
                    fallback_url,
                    format!("⚠ {err}"),
                )
            }
        };

        let mut app = Self {
            rt,
            client,
            tx,
            rx,
            lifecycle_task: None,
            panes: Vec::new(),
            pane_states: HashMap::new(),
            selected_id: None,
            input: String::new(),
            new_pane_name: String::new(),
            server_url,
            url_buf: settings.url,
            api_key_buf: settings.api_key,
            ca_cert_path_buf: settings.ca_cert_path,
            insecure_tls_buf: settings.insecure_tls,
            font_size: 13.0,
            status,
            terminal_focused: false,
            terminal_rect: None,
            resize_cols: 80,
            resize_rows: 24,
            history: Vec::new(),
            history_pos: None,
            history_draft: String::new(),
        };
        app.spawn_lifecycle_task();
        app
    }

    fn select_pane(&mut self, id: String, ctx: egui::Context) {
        if self.pane_states.contains_key(&id) {
            self.selected_id = Some(id);
            return;
        }
        let (rows, cols) = self
            .panes
            .iter()
            .find(|p| p.id == id)
            .map(|p| (p.rows, p.cols))
            .unwrap_or((24, 80));
        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, 1000)));
        let handle = self.rt.spawn(stream_pane_task(
            self.client.clone(),
            id.clone(),
            parser.clone(),
            self.tx.clone(),
            ctx,
        ));
        self.pane_states.insert(
            id.clone(),
            PaneState {
                parser,
                task: handle,
                throughput: ThroughputWindow::default(),
            },
        );
        self.selected_id = Some(id);
    }

    fn reconnect(&mut self) {
        let settings = ConnectionSettings {
            url: self.url_buf.trim().to_owned(),
            api_key: self.api_key_buf.trim().to_owned(),
            ca_cert_path: self.ca_cert_path_buf.trim().to_owned(),
            insecure_tls: self.insecure_tls_buf,
        };

        match build_client(&settings) {
            Ok(client) => {
                self.abort_background_tasks();
                self.server_url = settings.url;
                self.client = Arc::new(client);
                self.panes.clear();
                self.selected_id = None;
                self.spawn_lifecycle_task();
                self.status = format!("Connecting to {}...", self.server_url);
            }
            Err(err) => {
                self.status = format!("⚠ {err}");
            }
        }
    }

    fn spawn_lifecycle_task(&mut self) {
        if let Some(handle) = self.lifecycle_task.take() {
            handle.abort();
        }
        let handle = self
            .rt
            .spawn(lifecycle_task(self.client.clone(), self.tx.clone()));
        self.lifecycle_task = Some(handle);
    }

    fn abort_background_tasks(&mut self) {
        if let Some(handle) = self.lifecycle_task.take() {
            handle.abort();
        }
        for (_, state) in self.pane_states.drain() {
            state.task.abort();
        }
    }
}

// ── egui::App ─────────────────────────────────────────────────────────────────

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        apply_theme(ctx);

        if self.selected_id.is_some() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }

        if self.terminal_focused
            && ctx.input(|i| i.pointer.primary_clicked())
            && ctx
                .input(|i| i.pointer.interact_pos())
                .is_some_and(|pos| self.terminal_rect.is_none_or(|rect| !rect.contains(pos)))
        {
            self.terminal_focused = false;
        }

        // ── direct key input (must happen before any TextEdit is rendered) ────
        if self.terminal_focused {
            // Hold a virtual focus ID so no TextEdit thinks it has focus.
            ctx.memory_mut(|m| m.request_focus(egui::Id::new("terminal_direct")));

            let bytes: Vec<u8> = ctx.input_mut(|i| {
                let mut out = Vec::new();
                i.events.retain(|ev| match ev {
                    egui::Event::Text(t) => {
                        out.extend_from_slice(t.as_bytes());
                        false
                    }
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => {
                        if let Some(b) = key_to_pty_bytes(key, modifiers) {
                            out.extend_from_slice(&b);
                            false
                        } else {
                            true
                        }
                    }
                    _ => true,
                });
                out
            });

            if !bytes.is_empty() {
                if let Some(ref id) = self.selected_id.clone() {
                    let client = self.client.clone();
                    let id = id.clone();
                    let tx = self.tx.clone();
                    self.rt.spawn(async move {
                        if let Err(e) = client.send_input(&id, &bytes).await {
                            let _ = tx.send(Msg::Error(e.to_string()));
                        }
                    });
                }
            }
        }

        // ── drain messages ────────────────────────────────────────────────────
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::PanesUpdated(panes) => {
                    let removed_ids: Vec<String> = self
                        .pane_states
                        .keys()
                        .filter(|id| !panes.iter().any(|pane| &pane.id == *id))
                        .cloned()
                        .collect();
                    for id in removed_ids {
                        if let Some(state) = self.pane_states.remove(&id) {
                            state.task.abort();
                        }
                    }
                    if let Some(ref sel) = self.selected_id {
                        if !panes.iter().any(|p| &p.id == sel) {
                            self.selected_id = None;
                        }
                    }
                    self.panes = panes;
                    self.status.clear();
                }
                Msg::PaneOutput {
                    pane_id,
                    bytes,
                    received_at,
                } => {
                    if let Some(state) = self.pane_states.get_mut(&pane_id) {
                        state.throughput.record(received_at, bytes);
                    }
                }
                Msg::Error(e) => self.status = format!("⚠ {e}"),
            }
        }

        // ── collect UI actions ────────────────────────────────────────────────
        // Declared outside all closures so they can be processed after panels.
        let mut to_select: Option<String> = None;
        let mut to_delete: Option<String> = None;
        let mut to_create: Option<CreatePaneOptions> = None;
        let mut to_resize: Option<(u16, u16)> = None;
        let mut do_reconnect = false;

        // ── left panel: pane list ─────────────────────────────────────────────
        egui::SidePanel::left("panes_panel")
            .min_width(180.0)
            .max_width(260.0)
            .show(ctx, |ui| {
                ui.heading("biome_term");
                ui.separator();

                ui.label(egui::RichText::new("Connection").small().color(DIM_GREEN));
                let url_resp = ui.add(
                    egui::TextEdit::singleline(&mut self.url_buf)
                        .desired_width(f32::INFINITY)
                        .hint_text("https://host:3027"),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut self.api_key_buf)
                        .desired_width(f32::INFINITY)
                        .hint_text("API key")
                        .password(true),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut self.ca_cert_path_buf)
                        .desired_width(f32::INFINITY)
                        .hint_text("/path/to/ca.pem"),
                );
                ui.checkbox(
                    &mut self.insecure_tls_buf,
                    "Allow invalid TLS certs / hostnames",
                );
                if ui.button("Connect").clicked()
                    || (url_resp.lost_focus() && ctx.input(|i| i.key_pressed(egui::Key::Enter)))
                {
                    do_reconnect = true;
                }
                if self.server_url != self.url_buf.trim() {
                    ui.label(
                        egui::RichText::new(format!("Active: {}", self.server_url))
                            .small()
                            .color(DIM_GREEN),
                    );
                }

                if !self.status.is_empty() {
                    ui.label(egui::RichText::new(&self.status).small().color(GREEN));
                }

                ui.separator();

                // Pane list
                egui::ScrollArea::vertical()
                    .id_salt("pane_list")
                    .show(ui, |ui| {
                        for pane in &self.panes {
                            let label = pane.name.as_deref().unwrap_or(&pane.id[..8]);
                            let is_sel = self.selected_id.as_deref() == Some(&pane.id);
                            let color = if pane.terminated { DIM_GREEN } else { GREEN };
                            ui.horizontal(|ui| {
                                if ui
                                    .selectable_label(
                                        is_sel,
                                        egui::RichText::new(label).color(color),
                                    )
                                    .clicked()
                                {
                                    to_select = Some(pane.id.clone());
                                }
                                if ui.small_button("×").clicked() {
                                    to_delete = Some(pane.id.clone());
                                }
                            });
                        }
                    });

                ui.separator();

                // New pane row
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_pane_name)
                            .desired_width(100.0)
                            .hint_text("name"),
                    );
                    if ui.button("+").clicked() {
                        let name = self.new_pane_name.trim().to_owned();
                        to_create = Some(CreatePaneOptions {
                            cols: Some(200),
                            rows: Some(50),
                            name: if name.is_empty() { None } else { Some(name) },
                            shell: None,
                        });
                        self.new_pane_name.clear();
                    }
                });

                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Font");
                    ui.add(egui::Slider::new(&mut self.font_size, 9.0..=24.0).suffix("px"));
                });

                ui.separator();
                ui.label(egui::RichText::new("Resize").small().color(DIM_GREEN));
                ui.horizontal(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut self.resize_cols)
                            .range(10..=500)
                            .prefix("cols "),
                    );
                    ui.add(
                        egui::DragValue::new(&mut self.resize_rows)
                            .range(4..=200)
                            .prefix("rows "),
                    );
                });
                if ui.button("↔ Send resize").clicked() {
                    to_resize = Some((self.resize_cols, self.resize_rows));
                }
            });

        // ── process side-panel actions ────────────────────────────────────────
        if do_reconnect {
            self.reconnect();
        }
        if let Some(id) = to_select {
            self.select_pane(id, ctx.clone());
        }
        if let Some(id) = to_delete {
            let client = self.client.clone();
            let tx = self.tx.clone();
            self.rt.spawn(async move {
                if let Err(e) = client.delete_pane(&id).await {
                    let _ = tx.send(Msg::Error(e.to_string()));
                }
            });
        }
        if let Some(opts) = to_create {
            let client = self.client.clone();
            let tx = self.tx.clone();
            self.rt.spawn(async move {
                if let Err(e) = client.create_pane(opts).await {
                    let _ = tx.send(Msg::Error(e.to_string()));
                }
            });
        }
        if let Some((cols, rows)) = to_resize {
            if let Some(ref id) = self.selected_id.clone() {
                // Update local parser immediately so the display reflects the new size.
                if let Some(state) = self.pane_states.get(id) {
                    if let Ok(mut parser) = state.parser.lock() {
                        parser.screen_mut().set_size(rows, cols);
                    }
                }
                let client = self.client.clone();
                let id = id.clone();
                let tx = self.tx.clone();
                self.rt.spawn(async move {
                    if let Err(e) = client.resize_pane(&id, cols, rows).await {
                        let _ = tx.send(Msg::Error(e.to_string()));
                    }
                });
            }
        }

        // ── bottom panel: input ───────────────────────────────────────────────
        let mut to_send: Option<(String, String)> = None;
        let mut input_bar_clicked = false;

        egui::TopBottomPanel::bottom("input_panel").show(ctx, |ui| {
            if self.terminal_focused {
                // Show mode indicator; clicking here returns to typed-input mode.
                let resp = ui
                    .horizontal(|ui| {
                        ui.label(egui::RichText::new("⌨ DIRECT INPUT").color(GREEN).small());
                        ui.label(
                            egui::RichText::new("(click here for typed input)")
                                .color(DIM_GREEN)
                                .small(),
                        )
                    })
                    .response;
                if resp.clicked() {
                    input_bar_clicked = true;
                }
                // Also detect a plain click anywhere in the bar
                let bar_rect = ui.max_rect();
                if ctx.input(|i| {
                    i.pointer.primary_clicked()
                        && i.pointer
                            .interact_pos()
                            .map(|p| bar_rect.contains(p))
                            .unwrap_or(false)
                }) {
                    input_bar_clicked = true;
                }
            } else {
                let resp = ui
                    .horizontal(|ui| {
                        ui.label(egui::RichText::new(">").color(GREEN));
                        ui.add(
                            egui::TextEdit::singleline(&mut self.input)
                                .desired_width(f32::INFINITY)
                                .font(egui::FontId::monospace(self.font_size)),
                        )
                    })
                    .inner;

                if resp.has_focus() {
                    let up = ctx.input(|i| i.key_pressed(egui::Key::ArrowUp));
                    let down = ctx.input(|i| i.key_pressed(egui::Key::ArrowDown));

                    if up && !self.history.is_empty() {
                        if self.history_pos.is_none() {
                            self.history_draft = self.input.clone();
                            self.history_pos = Some(self.history.len() - 1);
                        } else if let Some(p) = self.history_pos {
                            if p > 0 {
                                self.history_pos = Some(p - 1);
                            }
                        }
                        if let Some(p) = self.history_pos {
                            self.input = self.history[p].clone();
                        }
                        // Move cursor to end of the restored entry.
                        let te_id = resp.id;
                        if let Some(mut state) = egui::TextEdit::load_state(ctx, te_id) {
                            state
                                .cursor
                                .set_char_range(Some(egui::text::CCursorRange::one(
                                    egui::text::CCursor::new(self.input.chars().count()),
                                )));
                            state.store(ctx, te_id);
                        }
                    }

                    if down {
                        match self.history_pos {
                            None => {}
                            Some(p) if p + 1 < self.history.len() => {
                                self.history_pos = Some(p + 1);
                                self.input = self.history[p + 1].clone();
                            }
                            _ => {
                                self.history_pos = None;
                                self.input = std::mem::take(&mut self.history_draft);
                            }
                        }
                        let te_id = resp.id;
                        if let Some(mut state) = egui::TextEdit::load_state(ctx, te_id) {
                            state
                                .cursor
                                .set_char_range(Some(egui::text::CCursorRange::one(
                                    egui::text::CCursor::new(self.input.chars().count()),
                                )));
                            state.store(ctx, te_id);
                        }
                    }
                }

                if resp.lost_focus() && ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
                    if let Some(ref id) = self.selected_id {
                        let raw = std::mem::take(&mut self.input);
                        // Push to history (skip blanks and consecutive dupes).
                        if !raw.trim().is_empty()
                            && self.history.last().map(|s| s.as_str()) != Some(raw.trim())
                        {
                            self.history.push(raw.trim().to_owned());
                        }
                        self.history_pos = None;
                        self.history_draft.clear();

                        let mut text = raw.replace('\n', "\r");
                        text.push('\r');
                        to_send = Some((id.clone(), text));
                    }
                    resp.request_focus();
                }
            }
        });

        if input_bar_clicked {
            self.terminal_focused = false;
        }

        if let Some((id, text)) = to_send {
            let client = self.client.clone();
            let tx = self.tx.clone();
            self.rt.spawn(async move {
                if let Err(e) = client.send_input(&id, text.as_bytes()).await {
                    let _ = tx.send(Msg::Error(e.to_string()));
                }
            });
        }

        // ── central panel: terminal ───────────────────────────────────────────
        let mut terminal_clicked = false;
        egui::CentralPanel::default().show(ctx, |ui| {
            // Detect clicks on the terminal area to enter direct-input mode.
            let panel_rect = ui.max_rect();
            self.terminal_rect = Some(panel_rect);
            if ctx.input(|i| {
                i.pointer.primary_clicked()
                    && i.pointer
                        .interact_pos()
                        .map(|p| panel_rect.contains(p))
                        .unwrap_or(false)
            }) {
                terminal_clicked = true;
            }

            if let Some(ref id) = self.selected_id.clone() {
                let throughput_label = self.selected_pane_throughput_label();
                if let Some(state) = self.pane_states.get(id) {
                    ui.horizontal(|ui| {
                        let pane_label = self
                            .panes
                            .iter()
                            .find(|pane| pane.id == *id)
                            .and_then(|pane| pane.name.as_deref())
                            .unwrap_or(&id[..8]);
                        ui.label(egui::RichText::new(pane_label).small().color(GREEN));
                        ui.separator();
                        ui.label(
                            egui::RichText::new(throughput_label)
                                .small()
                                .color(DIM_GREEN),
                        );
                    });
                    ui.add_space(6.0);
                    if let Ok(parser) = state.parser.lock() {
                        render_terminal(ui, &parser, self.font_size, self.terminal_focused);
                    }
                }
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        egui::RichText::new(
                            "Select a pane from the left panel, or press + to create one.",
                        )
                        .color(DIM_GREEN),
                    );
                });
            }
        });

        if terminal_clicked {
            self.terminal_focused = true;
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        BLACK.to_normalized_gamma_f32()
    }
}

impl App {
    fn selected_pane_throughput_label(&mut self) -> String {
        let Some(selected_id) = self.selected_id.clone() else {
            return "0 B/s (5s avg)".to_string();
        };

        let rate = self
            .pane_states
            .get_mut(&selected_id)
            .map(|state| state.throughput.bytes_per_second(Instant::now()))
            .unwrap_or(0.0);

        format!("{} /s (5s avg)", format_byte_rate(rate))
    }
}

// ── Terminal renderer ─────────────────────────────────────────────────────────

fn apply_theme(ctx: &egui::Context) {
    let visuals = terminal_visuals();
    ctx.set_theme(egui::Theme::Dark);
    ctx.set_visuals_of(egui::Theme::Dark, visuals.clone());
    ctx.set_visuals_of(egui::Theme::Light, visuals.clone());
}

fn terminal_visuals() -> egui::Visuals {
    let mut visuals = egui::Visuals::dark();
    visuals.override_text_color = Some(GREEN);
    visuals.panel_fill = BLACK;
    visuals.window_fill = BLACK;
    visuals.extreme_bg_color = BLACK;
    visuals.faint_bg_color = DARK_GREEN;
    visuals.code_bg_color = BLACK;
    visuals.hyperlink_color = GREEN;
    visuals.warn_fg_color = GREEN;
    visuals.error_fg_color = GREEN;
    visuals.selection.bg_fill = DARK_GREEN;
    visuals.selection.stroke = egui::Stroke::new(1.0, GREEN);

    visuals.widgets.noninteractive.bg_fill = BLACK;
    visuals.widgets.noninteractive.weak_bg_fill = BLACK;
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, DARK_GREEN);
    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, GREEN);

    visuals.widgets.inactive.bg_fill = BLACK;
    visuals.widgets.inactive.weak_bg_fill = BLACK;
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, DIM_GREEN);
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, GREEN);

    visuals.widgets.hovered.bg_fill = DARK_GREEN;
    visuals.widgets.hovered.weak_bg_fill = DARK_GREEN;
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, GREEN);
    visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, GREEN);

    visuals.widgets.active.bg_fill = DARK_GREEN;
    visuals.widgets.active.weak_bg_fill = DARK_GREEN;
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.5, GREEN);
    visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0, GREEN);

    visuals.widgets.open.bg_fill = BLACK;
    visuals.widgets.open.weak_bg_fill = BLACK;
    visuals.widgets.open.bg_stroke = egui::Stroke::new(1.0, GREEN);
    visuals.widgets.open.fg_stroke = egui::Stroke::new(1.0, GREEN);

    visuals
}

fn render_terminal(ui: &mut egui::Ui, parser: &vt100::Parser, font_size: f32, focused: bool) {
    const BG: egui::Color32 = BLACK;
    const FG: egui::Color32 = GREEN;

    let font_id = egui::FontId::monospace(font_size);
    let screen = parser.screen();
    let (rows, cols) = screen.size();
    let cursor = screen.cursor_position();

    let border_color = if focused {
        GREEN
    } else {
        egui::Color32::TRANSPARENT
    };

    egui::Frame {
        fill: BG,
        inner_margin: egui::Margin::same(6.0),
        stroke: egui::Stroke::new(if focused { 2.0 } else { 0.0 }, border_color),
        ..Default::default()
    }
    .show(ui, |ui| {
        egui::ScrollArea::both()
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;

                for row in 0..rows {
                    let mut job = egui::text::LayoutJob::default();
                    // Disable wrapping at the job level — ui.label() would
                    // override this, so we paint via a galley directly.
                    job.wrap.max_width = f32::INFINITY;

                    for col in 0..cols {
                        let (text, fg, bg) = if let Some(cell) = screen.cell(row, col) {
                            let raw = cell.contents();
                            let text = if raw.is_empty() {
                                " ".to_string()
                            } else {
                                raw.to_string()
                            };

                            let is_cursor = cursor == (row, col);
                            let mut fg = resolve_color(cell.fgcolor(), FG);
                            let mut bg = resolve_color(cell.bgcolor(), BG);

                            if cell.bold() {
                                fg = brighten(fg);
                            }
                            if cell.inverse() ^ is_cursor {
                                std::mem::swap(&mut fg, &mut bg);
                            } else if is_cursor {
                                bg = GREEN;
                                fg = BG;
                            }

                            (text, fg, bg)
                        } else {
                            (" ".to_string(), FG, BG)
                        };

                        job.append(
                            &text,
                            0.0,
                            egui::TextFormat {
                                font_id: font_id.clone(),
                                color: fg,
                                background: bg,
                                ..Default::default()
                            },
                        );
                    }

                    // Layout the job ourselves so wrap.max_width is honoured,
                    // then allocate exact space and paint — bypassing Label's
                    // width-clamping behaviour.
                    let galley = ui.fonts(|f| f.layout_job(job));
                    let size = galley.size();
                    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                    if ui.is_rect_visible(rect) {
                        ui.painter().galley(rect.min, galley, FG);
                    }
                }
            });
    });
}

fn format_byte_rate(bytes_per_second: f64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];

    let mut value = bytes_per_second.max(0.0);
    let mut unit = UNITS[0];

    for next_unit in UNITS.iter().skip(1) {
        if value < 1024.0 {
            break;
        }
        value /= 1024.0;
        unit = next_unit;
    }

    if unit == "B" {
        format!("{value:.0} {unit}")
    } else {
        format!("{value:.1} {unit}")
    }
}

fn resolve_color(color: vt100::Color, default: egui::Color32) -> egui::Color32 {
    match color {
        vt100::Color::Default => default,
        vt100::Color::Rgb(r, g, b) => egui::Color32::from_rgb(r, g, b),
        vt100::Color::Idx(i) => xterm256(i),
    }
}

fn brighten(c: egui::Color32) -> egui::Color32 {
    let [r, g, b, a] = c.to_array();
    let f = |v: u8| v.saturating_add(60);
    egui::Color32::from_rgba_premultiplied(f(r), f(g), f(b), a)
}

fn xterm256(idx: u8) -> egui::Color32 {
    #[rustfmt::skip]
    const ANSI: [(u8, u8, u8); 16] = [
        (0,0,0),       (128,0,0),   (0,128,0),   (128,128,0),
        (0,0,128),     (128,0,128), (0,128,128), (192,192,192),
        (128,128,128), (255,85,85), (85,255,85), (255,255,85),
        (85,85,255),   (255,85,255),(85,255,255),(255,255,255),
    ];
    match idx {
        0..=15 => {
            let (r, g, b) = ANSI[idx as usize];
            egui::Color32::from_rgb(r, g, b)
        }
        16..=231 => {
            let i = idx - 16;
            let b = i % 6;
            let g = (i / 6) % 6;
            let r = i / 36;
            let v = |x: u8| if x == 0 { 0u8 } else { x * 40 + 55 };
            egui::Color32::from_rgb(v(r), v(g), v(b))
        }
        _ => {
            let v = (idx - 232) * 10 + 8;
            egui::Color32::from_rgb(v, v, v)
        }
    }
}

// ── Key → PTY byte mapping ────────────────────────────────────────────────────

/// Convert a non-text egui key event to PTY bytes.
/// Returns `None` for regular printable keys (those arrive as `Event::Text`).
fn key_to_pty_bytes(key: &egui::Key, modifiers: &egui::Modifiers) -> Option<Vec<u8>> {
    // Ctrl+letter → control codes (only when Ctrl alone, no Alt)
    if modifiers.ctrl && !modifiers.alt {
        let byte: u8 = match key {
            egui::Key::A => 1,
            egui::Key::B => 2,
            egui::Key::C => 3,
            egui::Key::D => 4,
            egui::Key::E => 5,
            egui::Key::F => 6,
            egui::Key::G => 7,
            egui::Key::H => 8,
            egui::Key::I => 9,
            egui::Key::J => 10,
            egui::Key::K => 11,
            egui::Key::L => 12,
            egui::Key::M => 13,
            egui::Key::N => 14,
            egui::Key::O => 15,
            egui::Key::P => 16,
            egui::Key::Q => 17,
            egui::Key::R => 18,
            egui::Key::S => 19,
            egui::Key::T => 20,
            egui::Key::U => 21,
            egui::Key::V => 22,
            egui::Key::W => 23,
            egui::Key::X => 24,
            egui::Key::Y => 25,
            egui::Key::Z => 26,
            _ => return None,
        };
        return Some(vec![byte]);
    }

    // Special / non-printable keys (no modifier, or shift only)
    if modifiers.ctrl || modifiers.alt {
        return None;
    }

    let bytes: &[u8] = match key {
        egui::Key::Enter => b"\r",
        egui::Key::Backspace => b"\x7f",
        egui::Key::Delete => b"\x1b[3~",
        egui::Key::Escape => b"\x1b",
        egui::Key::Tab => b"\t",
        egui::Key::ArrowUp => b"\x1b[A",
        egui::Key::ArrowDown => b"\x1b[B",
        egui::Key::ArrowRight => b"\x1b[C",
        egui::Key::ArrowLeft => b"\x1b[D",
        egui::Key::Home => b"\x1b[H",
        egui::Key::End => b"\x1b[F",
        egui::Key::PageUp => b"\x1b[5~",
        egui::Key::PageDown => b"\x1b[6~",
        egui::Key::Insert => b"\x1b[2~",
        egui::Key::F1 => b"\x1bOP",
        egui::Key::F2 => b"\x1bOQ",
        egui::Key::F3 => b"\x1bOR",
        egui::Key::F4 => b"\x1bOS",
        egui::Key::F5 => b"\x1b[15~",
        egui::Key::F6 => b"\x1b[17~",
        egui::Key::F7 => b"\x1b[18~",
        egui::Key::F8 => b"\x1b[19~",
        egui::Key::F9 => b"\x1b[20~",
        egui::Key::F10 => b"\x1b[21~",
        egui::Key::F11 => b"\x1b[23~",
        egui::Key::F12 => b"\x1b[24~",
        _ => return None,
    };
    Some(bytes.to_vec())
}

fn env_connection_settings() -> ConnectionSettings {
    ConnectionSettings {
        url: env_string("BIOME_TERM_URL")
            .or_else(|| env_string("BIOME_URL"))
            .unwrap_or_else(|| "http://localhost:3021".to_string()),
        api_key: env_string("BIOME_API_KEY").unwrap_or_default(),
        ca_cert_path: env_string("BIOME_TLS_CA_CERT").unwrap_or_default(),
        insecure_tls: env_flag("BIOME_TLS_INSECURE"),
    }
}

fn initial_connection_settings() -> Result<ConnectionSettings, String> {
    apply_cli_overrides(env_connection_settings())
}

fn apply_cli_overrides(mut settings: ConnectionSettings) -> Result<ConnectionSettings, String> {
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--server" | "-server" | "--url" | "-url" => {
                settings.url = normalize_server_arg(next_arg_value(&mut args, &arg)?);
            }
            "--apikey" | "-apikey" | "--api-key" | "-api-key" => {
                settings.api_key = next_arg_value(&mut args, &arg)?;
            }
            "--cacert" | "-cacert" | "--ca-cert" | "-ca-cert" => {
                settings.ca_cert_path = next_arg_value(&mut args, &arg)?;
            }
            "--insecure" | "-insecure" => {
                settings.insecure_tls = true;
            }
            "--help" | "-help" | "-h" => {
                return Err(gui_usage());
            }
            _ => {
                return Err(format!("unknown argument: {arg}\n\n{}", gui_usage()));
            }
        }
    }

    Ok(settings)
}

fn next_arg_value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("missing value for {flag}\n\n{}", gui_usage()))
}

fn normalize_server_arg(server: String) -> String {
    let trimmed = server.trim();
    if trimmed.contains("://") {
        trimmed.to_owned()
    } else {
        format!("http://{trimmed}")
    }
}

fn gui_usage() -> String {
    [
        "Usage: biome-term-gui [-server HOST[:PORT]|URL] [-apikey KEY] [-cacert PATH] [-insecure]",
        "",
        "Examples:",
        "  biome-term-gui -server sg.orch.run:3021 -apikey mykey",
        "  biome-term-gui -server https://host.example:3027 -apikey mykey -cacert ca.pem",
        "",
        "Notes:",
        "  - If -server has no scheme, http:// is assumed.",
        "  - Use an explicit https:// URL when the remote endpoint is TLS-enabled.",
    ]
    .join("\n")
}

fn build_client(settings: &ConnectionSettings) -> Result<BiomeTermClient, String> {
    if settings.url.trim().is_empty() {
        return Err("server URL cannot be empty".to_string());
    }

    let mut builder = BiomeTermClient::builder(settings.url.trim());
    if !settings.api_key.trim().is_empty() {
        builder = builder.api_key(settings.api_key.trim());
    }
    if !settings.ca_cert_path.trim().is_empty() {
        builder = add_root_cert_from_path(builder, settings.ca_cert_path.trim())
            .map_err(|e| format!("failed to read CA certificate: {e}"))?;
    }
    if settings.insecure_tls {
        builder = builder
            .danger_accept_invalid_certs(true)
            .danger_accept_invalid_hostnames(true);
    }

    builder
        .build()
        .map_err(|e| format!("failed to build client: {e}"))
}

fn add_root_cert_from_path(
    builder: BiomeTermClientBuilder,
    path: &str,
) -> Result<BiomeTermClientBuilder, std::io::Error> {
    Ok(builder.add_root_certificate_pem(fs::read(path)?))
}

fn env_string(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn env_flag(name: &str) -> bool {
    env_string(name).is_some_and(|value| {
        matches!(
            value.to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

// ── Background tasks ──────────────────────────────────────────────────────────

async fn lifecycle_task(client: Arc<BiomeTermClient>, tx: std::sync::mpsc::SyncSender<Msg>) {
    if let Ok(panes) = client.list_panes().await {
        let _ = tx.send(Msg::PanesUpdated(panes));
    }
    match client.stream_lifecycle().await {
        Err(e) => {
            let _ = tx.send(Msg::Error(e.to_string()));
        }
        Ok(mut stream) => {
            while let Some(result) = stream.next().await {
                match result {
                    Ok(LifecycleEvent::Snapshot { panes }) => {
                        let _ = tx.send(Msg::PanesUpdated(panes));
                    }
                    Ok(LifecycleEvent::Created { .. } | LifecycleEvent::Deleted { .. }) => {
                        if let Ok(panes) = client.list_panes().await {
                            let _ = tx.send(Msg::PanesUpdated(panes));
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Msg::Error(e.to_string()));
                        break;
                    }
                }
            }
        }
    }
}

async fn stream_pane_task(
    client: Arc<BiomeTermClient>,
    id: String,
    parser: Arc<Mutex<vt100::Parser>>,
    tx: std::sync::mpsc::SyncSender<Msg>,
    ctx: egui::Context,
) {
    match client.stream_pane_screen_diff(&id).await {
        Err(e) => {
            let _ = tx.send(Msg::Error(e.to_string()));
        }
        Ok(mut stream) => {
            while let Some(result) = stream.next().await {
                match result {
                    Ok(event) => {
                        let bytes = event.data.len();
                        parser.lock().unwrap().process(&event.data);
                        let _ = tx.send(Msg::PaneOutput {
                            pane_id: id.clone(),
                            bytes,
                            received_at: Instant::now(),
                        });
                        ctx.request_repaint();
                    }
                    Err(e) => {
                        let _ = tx.send(Msg::Error(e.to_string()));
                        break;
                    }
                }
            }
        }
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() -> eframe::Result<()> {
    let settings = match initial_connection_settings() {
        Ok(settings) => settings,
        Err(message) => {
            if message.starts_with("Usage: ") {
                println!("{message}");
                std::process::exit(0);
            }
            eprintln!("{message}");
            std::process::exit(2);
        }
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("biome_term")
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([600.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native(
        "biome_term",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, settings.clone())))),
    )
}

#[cfg(test)]
mod tests {
    use super::{format_byte_rate, ThroughputWindow};
    use std::time::{Duration, Instant};

    #[test]
    fn throughput_window_averages_only_the_last_five_seconds() {
        let start = Instant::now();
        let mut window = ThroughputWindow::default();

        window.record(start, 100);
        window.record(start + Duration::from_secs(2), 50);

        assert_eq!(
            window.bytes_per_second(start + Duration::from_secs(4)),
            30.0
        );

        window.record(start + Duration::from_secs(6), 200);

        assert_eq!(
            window.bytes_per_second(start + Duration::from_secs(6)),
            50.0
        );
        assert_eq!(
            window.bytes_per_second(start + Duration::from_secs(12)),
            0.0
        );
    }

    #[test]
    fn format_byte_rate_uses_binary_units() {
        assert_eq!(format_byte_rate(999.0), "999 B");
        assert_eq!(format_byte_rate(2048.0), "2.0 KiB");
        assert_eq!(format_byte_rate(1_572_864.0), "1.5 MiB");
    }
}
