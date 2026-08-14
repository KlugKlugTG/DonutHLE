#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioFormat {
    pub sample_rate: u32,
    pub channels: u16,
}

#[derive(Debug)]
pub struct AudioMixer {
    format: AudioFormat,
    samples: Vec<i16>,
}

impl AudioMixer {
    pub fn new(format: AudioFormat) -> Self {
        Self {
            format,
            samples: Vec::new(),
        }
    }
    pub fn format(&self) -> AudioFormat {
        self.format
    }
    pub fn push_pcm16(&mut self, samples: &[i16]) {
        self.samples.extend_from_slice(samples);
    }
    pub fn drain(&mut self, max_samples: usize) -> Vec<i16> {
        let count = max_samples.min(self.samples.len());
        self.samples.drain(..count).collect()
    }
    pub fn queued_samples(&self) -> usize {
        self.samples.len()
    }
}
