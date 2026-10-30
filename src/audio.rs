//! Click sounds.
//!
//! There is no audio library dependency: a short click is synthesised into a
//! WAV file and played by whatever sound player the system has (PipeWire,
//! PulseAudio or ALSA). Players are spawned per click and reaped lazily.

use crate::config::Sound;
use crate::input::Button;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

const RATE: u32 = 44_100;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Player {
    PwPlay,
    Paplay,
    Aplay,
}

pub struct Audio {
    player: Option<Player>,
    dir: PathBuf,
    children: Vec<Child>,
    warned: bool,
}

fn have(cmd: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(cmd).is_file()))
}

/// 16-bit mono WAV of a short click.
pub fn synth_click(button: Button, pressed: bool, volume: f32) -> Vec<u8> {
    let (freq, decay, len_ms, amp) = match (pressed, button) {
        (true, Button::Right) => (1050.0f32, 0.011f32, 70.0f32, 0.85f32),
        (true, Button::Middle) => (1900.0, 0.009, 60.0, 0.8),
        (true, _) => (1500.0, 0.010, 65.0, 0.85),
        (false, _) => (2500.0, 0.005, 35.0, 0.45),
    };
    let n = (RATE as f32 * len_ms / 1000.0) as usize;
    let mut rng = 0x2545_F491u32;
    let mut pcm = Vec::with_capacity(n * 2);
    for i in 0..n {
        let t = i as f32 / RATE as f32;
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        let noise = (rng as f32 / u32::MAX as f32) * 2.0 - 1.0;
        let tone = (std::f32::consts::TAU * freq * t).sin() * (-t / decay).exp();
        let tick = noise * (-t / 0.0014).exp() * 0.35;
        let fade_out = ((n - i) as f32 / (RATE as f32 * 0.004)).min(1.0);
        let s = ((tone * 0.55 + tick) * amp * volume.clamp(0.0, 1.0) * fade_out).clamp(-1.0, 1.0);
        pcm.extend_from_slice(&((s * 32767.0) as i16).to_le_bytes());
    }
    let mut wav = Vec::with_capacity(44 + pcm.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    wav.extend_from_slice(&pcm);
    wav
}

impl Default for Audio {
    fn default() -> Self {
        Self::new()
    }
}

impl Audio {
    pub fn new() -> Self {
        let player = if have("pw-play") {
            Some(Player::PwPlay)
        } else if have("paplay") {
            Some(Player::Paplay)
        } else if have("aplay") {
            Some(Player::Aplay)
        } else {
            None
        };
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
            .unwrap_or_else(std::env::temp_dir)
            .join("sourizz");
        Self { player, dir, children: Vec::new(), warned: false }
    }

    fn wav_for(&self, button: Button, pressed: bool, volume: f32) -> Option<PathBuf> {
        let kind = match (pressed, button) {
            (false, _) => "up",
            (true, Button::Right) => "right",
            (true, Button::Middle) => "middle",
            (true, _) => "left",
        };
        let path = self.dir.join(format!("click-{kind}-{}.wav", (volume * 100.0).round() as u32));
        if !path.is_file() {
            std::fs::create_dir_all(&self.dir).ok()?;
            std::fs::write(&path, synth_click(button, pressed, volume)).ok()?;
        }
        Some(path)
    }

    pub fn play(&mut self, cfg: &Sound, button: Button, pressed: bool) {
        self.children.retain_mut(|c| matches!(c.try_wait(), Ok(None)));
        if self.children.len() >= 6 {
            return; // a burst of clicks: skipping some is better than a fork storm
        }
        let Some(player) = self.player else {
            if !self.warned {
                log::warn!("click sound enabled but no player found (install pipewire-bin, pulseaudio-utils or alsa-utils)");
                self.warned = true;
            }
            return;
        };
        let custom = (!cfg.file.is_empty()).then(|| PathBuf::from(&cfg.file)).filter(|p| p.is_file());
        let file = match custom.clone().or_else(|| self.wav_for(button, pressed, cfg.volume)) {
            Some(f) => f,
            None => return,
        };
        // The built-in WAV already has the volume baked in; custom files get player-side volume.
        let vol = if custom.is_some() { cfg.volume } else { 1.0 };
        let mut cmd = match player {
            Player::PwPlay => {
                let mut c = Command::new("pw-play");
                c.arg("--volume").arg(format!("{vol:.2}")).arg(&file);
                c
            }
            Player::Paplay => {
                let mut c = Command::new("paplay");
                c.arg(format!("--volume={}", (vol * 65536.0) as u32)).arg(&file);
                c
            }
            Player::Aplay => {
                let mut c = Command::new("aplay");
                c.arg("-q").arg(&file);
                c
            }
        };
        cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        match cmd.spawn() {
            Ok(child) => self.children.push(child),
            Err(e) => log::warn!("cannot start {player:?}: {e}"),
        }
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        for c in &mut self.children {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_is_well_formed() {
        let w = synth_click(Button::Left, true, 0.5);
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(&w[8..12], b"WAVE");
        let data_len = u32::from_le_bytes(w[40..44].try_into().unwrap()) as usize;
        assert_eq!(w.len(), 44 + data_len);
        assert_eq!(u32::from_le_bytes(w[4..8].try_into().unwrap()) as usize, w.len() - 8);
    }

    fn peak(w: &[u8]) -> i16 {
        w[44..].chunks(2).map(|b| i16::from_le_bytes([b[0], b[1]]).abs()).max().unwrap()
    }

    #[test]
    fn volume_scales_amplitude_and_zero_is_silent() {
        let loud = peak(&synth_click(Button::Left, true, 1.0));
        let quiet = peak(&synth_click(Button::Left, true, 0.25));
        assert!(loud > 2 * quiet, "{loud} vs {quiet}");
        assert_eq!(peak(&synth_click(Button::Left, true, 0.0)), 0);
        assert!(loud < 32767);
    }
}
