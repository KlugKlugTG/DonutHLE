use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputEvent {
    TouchDown { id: u32, x: f32, y: f32 },
    TouchMove { id: u32, x: f32, y: f32 },
    TouchUp { id: u32, x: f32, y: f32 },
    KeyDown { key: u32 },
    KeyUp { key: u32 },
}

#[derive(Debug, Default)]
pub struct InputQueue {
    events: VecDeque<InputEvent>,
}

impl InputQueue {
    pub fn push(&mut self, event: InputEvent) {
        self.events.push_back(event);
    }

    pub fn pop(&mut self) -> Option<InputEvent> {
        self.events.pop_front()
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}
