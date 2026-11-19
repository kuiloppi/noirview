use anyhow::{Context, Result};
use rand::seq::SliceRandom;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};
use std::{fs::File, io::BufReader, path::PathBuf};
pub struct Music {
    stream: Option<(OutputStream, OutputStreamHandle)>,
    sink: Option<Sink>,
    playlist: Vec<PathBuf>,
    index: usize,
    pub playing: bool,
    pub error: Option<String>,
    shuffle: bool,
    volume: f32,
}
impl Default for Music {
    fn default() -> Self {
        Self {
            stream: None,
            sink: None,
            playlist: vec![],
            index: 0,
            playing: false,
            error: None,
            shuffle: false,
            volume: 0.7,
        }
    }
}
impl Music {
    pub fn configure(&mut self, paths: &[PathBuf], shuffle: bool, volume: f32) {
        if self.playlist != paths {
            self.stop();
            self.playlist = paths.to_vec();
            self.index = 0;
        }
        self.shuffle = shuffle;
        self.volume = volume;
        if let Some(s) = &self.sink {
            s.set_volume(volume);
        }
    }
    pub fn toggle(&mut self) {
        if self.playing {
            if let Some(s) = &self.sink {
                s.pause();
            }
            self.playing = false;
        } else if let Some(s) = &self.sink {
            s.play();
            self.playing = true;
        } else if !self.playlist.is_empty() {
            if self.shuffle {
                self.index = rand::random::<usize>() % self.playlist.len();
            }
            if let Err(e) = self.load() {
                self.error = Some(format!("{e:#}"));
            }
        }
    }
    fn load(&mut self) -> Result<()> {
        if self.stream.is_none() {
            self.stream = Some(OutputStream::try_default().context("No audio output device")?);
        }
        let stream = self.stream.as_ref().unwrap();
        let sink = Sink::try_new(&stream.1)?;
        let path = self.playlist.get(self.index).context("No track selected")?;
        let decoder = Decoder::new(BufReader::new(File::open(path)?))
            .with_context(|| format!("Cannot decode {}", path.display()))?;
        sink.append(decoder);
        sink.set_volume(self.volume);
        self.sink = Some(sink);
        self.playing = true;
        Ok(())
    }
    pub fn tick(&mut self) {
        if self.playing && self.sink.as_ref().map(|s| s.empty()).unwrap_or(false) {
            if self.playlist.is_empty() {
                self.stop();
                return;
            }
            self.index = if self.shuffle {
                let choices: Vec<_> = (0..self.playlist.len())
                    .filter(|i| self.playlist.len() == 1 || *i != self.index)
                    .collect();
                *choices.choose(&mut rand::thread_rng()).unwrap()
            } else {
                (self.index + 1) % self.playlist.len()
            };
            if let Err(e) = self.load() {
                self.stop();
                self.error = Some(format!("{e:#}"));
            }
        }
    }
    fn stop(&mut self) {
        if let Some(s) = self.sink.take() {
            s.stop();
        }
        self.playing = false;
    }
    pub fn track(&self) -> Option<&str> {
        self.playlist.get(self.index)?.file_name()?.to_str()
    }
}
