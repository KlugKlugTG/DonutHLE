#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intent {
    pub action: Option<String>,
    pub categories: Vec<String>,
    pub component: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivityState {
    Created,
    Resumed,
    Paused,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityRecord {
    pub name: String,
    pub state: ActivityState,
    pub last_intent: Option<Intent>,
}

#[derive(Debug, Default)]
pub struct ActivityManager {
    stack: Vec<ActivityRecord>,
}

impl ActivityManager {
    pub fn start_activity(&mut self, name: String, intent: Intent) {
        self.stack.push(ActivityRecord {
            name,
            state: ActivityState::Resumed,
            last_intent: Some(intent),
        });
    }
    pub fn pause_top(&mut self) {
        if let Some(activity) = self.stack.last_mut() {
            activity.state = ActivityState::Paused;
        }
    }
    pub fn resume_top(&mut self) {
        if let Some(activity) = self.stack.last_mut() {
            activity.state = ActivityState::Resumed;
        }
    }
    pub fn finish_top(&mut self) -> Option<ActivityRecord> {
        self.stack.pop()
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
}
