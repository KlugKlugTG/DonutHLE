use std::collections::{HashMap, VecDeque};

use crate::assets::AssetStore;
use crate::HostGles;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Int(i32),
    Long(i64),
    Float(f32),
    Bool(bool),
    String(String),
}

impl Eq for Value {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intent {
    pub action: Option<String>,
    pub categories: Vec<String>,
    pub component: Option<String>,
    pub extras: HashMap<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityState {
    Created,
    Started,
    Resumed,
    Paused,
    Stopped,
    Destroyed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityRecord {
    pub name: String,
    pub state: ActivityState,
    pub last_intent: Option<Intent>,
    pub saved_instance_state: HashMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleEvent {
    Create,
    Start,
    Resume,
    Pause,
    Stop,
    Destroy,
    NewIntent(Intent),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub target: Option<String>,
    pub what: i32,
    pub arg1: i32,
    pub arg2: i32,
    pub obj: Value,
}

#[derive(Debug, Clone, Default)]
pub struct ActivityManager {
    stack: Vec<ActivityRecord>,
    lifecycle: VecDeque<(String, LifecycleEvent)>,
}

impl ActivityManager {
    pub fn start_activity(&mut self, name: String, intent: Intent) {
        if let Some(top) = self.stack.last_mut() {
            top.state = ActivityState::Paused;
            self.lifecycle
                .push_back((top.name.clone(), LifecycleEvent::Pause));
        }
        self.stack.push(ActivityRecord {
            name: name.clone(),
            state: ActivityState::Created,
            last_intent: Some(intent),
            saved_instance_state: HashMap::new(),
        });
        self.lifecycle
            .push_back((name.clone(), LifecycleEvent::Create));
        if let Some(top) = self.stack.last_mut() {
            top.state = ActivityState::Started;
        }
        self.lifecycle
            .push_back((name.clone(), LifecycleEvent::Start));
        if let Some(top) = self.stack.last_mut() {
            top.state = ActivityState::Resumed;
        }
        self.lifecycle.push_back((name, LifecycleEvent::Resume));
    }

    pub fn deliver_new_intent(&mut self, intent: Intent) {
        if let Some(top) = self.stack.last_mut() {
            top.last_intent = Some(intent.clone());
            self.lifecycle
                .push_back((top.name.clone(), LifecycleEvent::NewIntent(intent)));
        }
    }

    pub fn pause_top(&mut self) {
        if let Some(activity) = self.stack.last_mut() {
            activity.state = ActivityState::Paused;
            self.lifecycle
                .push_back((activity.name.clone(), LifecycleEvent::Pause));
        }
    }

    pub fn resume_top(&mut self) {
        if let Some(activity) = self.stack.last_mut() {
            activity.state = ActivityState::Resumed;
            self.lifecycle
                .push_back((activity.name.clone(), LifecycleEvent::Resume));
        }
    }

    pub fn finish_top(&mut self) -> Option<ActivityRecord> {
        let mut record = self.stack.pop()?;
        record.state = ActivityState::Destroyed;
        self.lifecycle
            .push_back((record.name.clone(), LifecycleEvent::Destroy));
        if let Some(top) = self.stack.last_mut() {
            top.state = ActivityState::Resumed;
            self.lifecycle
                .push_back((top.name.clone(), LifecycleEvent::Resume));
        }
        Some(record)
    }

    pub fn top(&self) -> Option<&ActivityRecord> {
        self.stack.last()
    }

    pub fn len(&self) -> usize {
        self.stack.len()
    }

    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    pub fn drain_lifecycle(&mut self) -> impl Iterator<Item = (String, LifecycleEvent)> + '_ {
        self.lifecycle.drain(..)
    }
}

#[derive(Debug, Default)]
pub struct MessageQueue {
    messages: VecDeque<Message>,
}

impl MessageQueue {
    pub fn post(&mut self, message: Message) {
        self.messages.push_back(message);
    }

    pub fn dequeue(&mut self) -> Option<Message> {
        self.messages.pop_front()
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }
}

#[derive(Debug, Default)]
pub struct ResourceRegistry {
    values: HashMap<u32, Value>,
}

impl ResourceRegistry {
    pub fn insert(&mut self, id: u32, value: Value) {
        self.values.insert(id, value);
    }

    pub fn get(&self, id: u32) -> Option<&Value> {
        self.values.get(&id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewNode {
    pub id: i32,
    pub class_name: String,
    pub text: Option<String>,
    pub bounds: Rect,
    pub visible: bool,
    pub children: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn contains(self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewEvent {
    Click(i32),
    TouchDown { x: i32, y: i32 },
    TouchUp { x: i32, y: i32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameworkCall {
    GetSystemService {
        name: String,
    },
    GetSharedPreferences {
        name: String,
        mode: i32,
    },
    SharedPreferencesGetString {
        prefs: u32,
        key: String,
        default: String,
    },
    SharedPreferencesPutString {
        prefs: u32,
        key: String,
        value: String,
    },
    SurfaceCreated {
        surface: u32,
    },
    SurfaceChanged {
        surface: u32,
        format: i32,
        width: i32,
        height: i32,
    },
    SurfaceDestroyed {
        surface: u32,
    },
    AudioTrackWrite {
        track: u32,
        samples: i32,
    },
    MediaPlayerPrepare {
        player: u32,
    },
    MediaPlayerStart {
        player: u32,
    },
    MediaPlayerStop {
        player: u32,
    },
    SoundPoolPlay {
        pool: u32,
        sound: i32,
        left: i32,
        right: i32,
        loop_count: i32,
        rate: i32,
    },
    SensorRegister {
        sensor: u32,
    },
    NetworkRequest {
        url: String,
    },
    NewView {
        class_name: String,
    },
    SetViewId {
        view: u32,
        id: i32,
    },
    SetViewText {
        view: u32,
        text: String,
    },
    AddLayerChild {
        parent: u32,
        child: u32,
    },
    AddView {
        parent: u32,
        child: u32,
    },
    SetContentView {
        activity: u32,
        view: u32,
    },
    FinishActivity {
        activity: u32,
    },
    GetString {
        id: u32,
    },
    Log {
        priority: i32,
        tag: String,
        message: String,
    },
    Toast {
        text: String,
        duration: i32,
    },
    PostMessage(Message),
}

#[derive(Debug, Clone, PartialEq)]
pub enum FrameworkResult {
    Void,
    Int(i32),
    Long(i64),
    Float(f32),
    Bool(bool),
    Object(u32),
    String(String),
}

impl Eq for FrameworkResult {}

#[derive(Debug, Default)]
pub struct Framework {
    pub activities: ActivityManager,
    pub messages: MessageQueue,
    pub resources: ResourceRegistry,
    pub views: HashMap<u32, ViewNode>,
    pub logs: Vec<String>,
    pub toasts: Vec<String>,
    pub content_views: HashMap<u32, u32>,
    pub preferences: HashMap<(String, String), String>,
    pub surface_events: Vec<String>,
    pub audio_writes: usize,
    pub system_services: HashMap<String, u32>,
    pub gles: HostGles,
    pub gdx_listener: Option<u32>,
    pub gdx_view: Option<u32>,
    pub gdx_graphics: Option<u32>,
    pub gdx_audio: Option<u32>,
    pub gdx_files: Option<u32>,
    pub gdx_input: Option<u32>,
    pub assets: Option<AssetStore>,
    pub surface_size: (i32, i32),
    next_handle: u32,
}

impl Framework {
    pub fn new() -> Self {
        Self {
            next_handle: 1,
            gles: HostGles::default(),
            assets: Some(AssetStore::default()),
            ..Self::default()
        }
    }

    pub fn alloc_view(&mut self, class_name: impl Into<String>) -> u32 {
        let handle = self.next_handle;
        self.next_handle = self.next_handle.saturating_add(1);
        self.views.insert(
            handle,
            ViewNode {
                id: -1,
                class_name: class_name.into(),
                text: None,
                bounds: Rect {
                    left: 0,
                    top: 0,
                    right: 320,
                    bottom: 480,
                },
                visible: true,
                children: Vec::new(),
            },
        );
        handle
    }

    pub fn ensure_view(&mut self, handle: u32, class_name: impl Into<String>) {
        self.views.entry(handle).or_insert_with(|| ViewNode {
            id: -1,
            class_name: class_name.into(),
            text: None,
            bounds: Rect {
                left: 0,
                top: 0,
                right: 320,
                bottom: 480,
            },
            visible: true,
            children: Vec::new(),
        });
    }

    pub fn dispatch(&mut self, call: FrameworkCall) -> Result<FrameworkResult, String> {
        match call {
            FrameworkCall::GetSystemService { name } => {
                let handle = if let Some(handle) = self.system_services.get(&name) {
                    *handle
                } else {
                    let handle = self.next_handle;
                    self.next_handle = self.next_handle.saturating_add(1);
                    self.system_services.insert(name, handle);
                    handle
                };
                Ok(FrameworkResult::Object(handle))
            }
            FrameworkCall::GetSharedPreferences { name, .. } => {
                let handle = self.next_handle;
                self.next_handle = self.next_handle.saturating_add(1);
                self.system_services.insert(format!("prefs:{name}"), handle);
                Ok(FrameworkResult::Object(handle))
            }
            FrameworkCall::SharedPreferencesGetString { key, default, .. } => {
                Ok(FrameworkResult::String(
                    self.preferences
                        .iter()
                        .find(|((_, k), _)| k == &key)
                        .map(|(_, v)| v.clone())
                        .unwrap_or(default),
                ))
            }
            FrameworkCall::SharedPreferencesPutString { key, value, .. } => {
                self.preferences.insert(("default".to_owned(), key), value);
                Ok(FrameworkResult::Void)
            }
            FrameworkCall::SurfaceCreated { surface } => {
                self.surface_events.push(format!("created:{surface}"));
                Ok(FrameworkResult::Void)
            }
            FrameworkCall::SurfaceChanged {
                surface,
                format,
                width,
                height,
            } => {
                self.surface_events
                    .push(format!("changed:{surface}:{format}:{width}x{height}"));
                Ok(FrameworkResult::Void)
            }
            FrameworkCall::SurfaceDestroyed { surface } => {
                self.surface_events.push(format!("destroyed:{surface}"));
                Ok(FrameworkResult::Void)
            }
            FrameworkCall::AudioTrackWrite { samples, .. } => {
                self.audio_writes = self.audio_writes.saturating_add(samples.max(0) as usize);
                Ok(FrameworkResult::Int(samples))
            }
            FrameworkCall::MediaPlayerPrepare { .. }
            | FrameworkCall::MediaPlayerStart { .. }
            | FrameworkCall::MediaPlayerStop { .. }
            | FrameworkCall::SoundPoolPlay { .. } => Ok(FrameworkResult::Void),
            FrameworkCall::SensorRegister { .. } => Ok(FrameworkResult::Bool(true)),
            FrameworkCall::NetworkRequest { .. } => Ok(FrameworkResult::Object(0)),
            FrameworkCall::NewView { class_name } => {
                Ok(FrameworkResult::Object(self.alloc_view(class_name)))
            }
            FrameworkCall::SetViewId { view, id } => {
                self.views
                    .get_mut(&view)
                    .ok_or_else(|| format!("unknown view {view}"))?
                    .id = id;
                Ok(FrameworkResult::Void)
            }
            FrameworkCall::SetViewText { view, text } => {
                self.views
                    .get_mut(&view)
                    .ok_or_else(|| format!("unknown view {view}"))?
                    .text = Some(text);
                Ok(FrameworkResult::Void)
            }
            FrameworkCall::AddLayerChild { parent, child } => {
                self.ensure_view(parent, "Lcom/hyperkani/common/Layer;");
                self.ensure_view(child, "Lcom/hyperkani/common/Layer;");
                if let Some(view) = self.views.get_mut(&parent) {
                    view.children.push(child);
                }
                Ok(FrameworkResult::Void)
            }
            FrameworkCall::AddView { parent, child } => {
                self.views
                    .get_mut(&parent)
                    .ok_or_else(|| format!("unknown parent view {parent}"))?
                    .children
                    .push(child);
                Ok(FrameworkResult::Void)
            }
            FrameworkCall::SetContentView { activity, view } => {
                if !self.views.contains_key(&view) {
                    return Err(format!("unknown content view {view}"));
                }
                self.content_views.insert(activity, view);
                Ok(FrameworkResult::Void)
            }
            FrameworkCall::FinishActivity { .. } => {
                self.activities.finish_top();
                Ok(FrameworkResult::Void)
            }
            FrameworkCall::GetString { id } => match self.resources.get(id) {
                Some(Value::String(value)) => Ok(FrameworkResult::String(value.clone())),
                Some(Value::Int(value)) => Ok(FrameworkResult::String(value.to_string())),
                _ => Err(format!("resource string {id:#x} is unavailable")),
            },
            FrameworkCall::Log {
                priority,
                tag,
                message,
            } => {
                self.logs.push(format!("{priority}/{tag}: {message}"));
                Ok(FrameworkResult::Int(message.len() as i32))
            }
            FrameworkCall::Toast { text, duration } => {
                self.toasts.push(format!("{duration}:{text}"));
                Ok(FrameworkResult::Object(0))
            }
            FrameworkCall::PostMessage(message) => {
                self.messages.post(message);
                Ok(FrameworkResult::Void)
            }
        }
    }
}
