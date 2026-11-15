//! Live VVVF audition through the audio output (feature `audio`, cpal).
//!
//! The GUI thread updates an [`AudioDrive`] snapshot each frame; the cpal
//! callback renders either the C++ VVVF synthesiser or the BVE real-sound
//! playback engine (loop resampling driven by the motor data curves).
//!
//! The output stream itself is opened on a background thread by [`AudioInit`]:
//! audio stacks can hang or take a long time in odd environments (WASAPI with
//! no audio endpoint, e.g. remote-desktop sessions), which must never stop the
//! first window from appearing. Set `MRPC_NO_AUDIO=1` to skip audio entirely.

#[cfg(feature = "audio")]
mod imp {
    use std::sync::{Arc, Mutex};

    use mrpc_core::bve_playback::BvePlaybackEngine;
    use mrpc_vvvf::{Engine, VvvfParams};

    /// What the audio callback should render.
    #[derive(Clone)]
    pub enum AudioDrive {
        /// C++ VVVF synthesiser.
        Synth {
            params: VvvfParams,
            fundamental_hz: f64,
            power: f64,
        },
        /// BVE real-sound playback: per-channel playback-rate multipliers and
        /// gains (computed from the motor data curves on the GUI thread).
        Bve { rates: Vec<f32>, gains: Vec<f32> },
    }

    #[derive(Clone)]
    struct PlaybackParams {
        drive: AudioDrive,
        playing: bool,
    }

    struct Shared {
        state: Mutex<PlaybackParams>,
        bve: Mutex<Option<Arc<Mutex<BvePlaybackEngine>>>>,
    }

    pub struct AudioOut {
        shared: Arc<Shared>,
        _stream: cpal::Stream,
    }

    impl AudioOut {
        pub fn try_start() -> Result<AudioOut, String> {
            use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

            let host = cpal::default_host();
            let device = host
                .default_output_device()
                .ok_or_else(|| "no output device".to_string())?;
            let config = device
                .default_output_config()
                .map_err(|e| e.to_string())?;
            // cpal 0.18: `SampleRate` is a plain `u32` type alias.
            let sample_rate = config.sample_rate() as f64;
            let channels = config.channels() as usize;
            let stream_config = cpal::StreamConfig {
                channels: config.channels(),
                sample_rate: config.sample_rate(),
                buffer_size: cpal::BufferSize::Default,
            };

            let shared = Arc::new(Shared {
                state: Mutex::new(PlaybackParams {
                    drive: AudioDrive::Synth {
                        params: VvvfParams {
                            sample_rate,
                            ..Default::default()
                        },
                        fundamental_hz: 0.0,
                        power: 0.0,
                    },
                    playing: false,
                }),
                bve: Mutex::new(None),
            });

            let shared_cb = shared.clone();
            let err_fn = |err| log::warn!("[audio] stream error: {err}");
            // cpal 0.18 takes the stream config by value.
            let stream = device
                .build_output_stream(
                    stream_config,
                    move |data: &mut [f32], _info: &cpal::OutputCallbackInfo| {
                        let snapshot = {
                            let guard = shared_cb.state.lock().unwrap();
                            guard.clone()
                        };
                        if !snapshot.playing {
                            for d in data.iter_mut() {
                                *d = 0.0;
                            }
                            return;
                        }
                        let frames = data.len() / channels.max(1);
                        if frames == 0 {
                            return;
                        }
                        let mut mono = vec![0f32; frames];
                        match &snapshot.drive {
                            AudioDrive::Synth { params, fundamental_hz, power } => {
                                // One engine per callback keeps phase continuity
                                // cheap; acceptable for a preview tool.
                                let mut engine = Engine::new(*params);
                                engine.render(*fundamental_hz, *power, &mut mono);
                            }
                            AudioDrive::Bve { rates, gains } => {
                                let guard = shared_cb.bve.lock().unwrap();
                                if let Some(engine) = guard.as_ref() {
                                    let mut engine = engine.lock().unwrap();
                                    engine.render(rates, gains, &mut mono);
                                }
                            }
                        }
                        for (i, frame) in data.chunks_mut(channels.max(1)).enumerate() {
                            let s = mono.get(i).copied().unwrap_or(0.0);
                            for d in frame.iter_mut() {
                                *d = s;
                            }
                        }
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| e.to_string())?;
            stream.play().map_err(|e| e.to_string())?;

            Ok(AudioOut { shared, _stream: stream })
        }

        pub fn set_playing(&mut self, playing: bool) {
            self.shared.state.lock().unwrap().playing = playing;
        }

        /// Install the BVE playback engine used by [`AudioDrive::Bve`].
        pub fn set_bve_engine(&mut self, engine: BvePlaybackEngine) {
            *self.shared.bve.lock().unwrap() = Some(Arc::new(Mutex::new(engine)));
        }

        pub fn update(&mut self, drive: AudioDrive) {
            let mut guard = self.shared.state.lock().unwrap();
            guard.drive = drive;
        }
    }

    /// Outcome of the background audio startup, surfaced in the UI.
    #[derive(Clone)]
    pub enum AudioStatus {
        /// Still opening the output device.
        Starting,
        /// The output stream is live.
        Ready,
        /// Audio is unusable in this environment (human-readable reason).
        Failed(String),
    }

    /// Handle to the audio output, initialised on a background thread.
    ///
    /// Mirrors the methods of [`AudioOut`]; they are no-ops until the stream
    /// is ready, so the GUI never has to wait for (or fail because of) audio.
    pub struct AudioInit {
        out: Arc<Mutex<Option<AudioOut>>>,
        status: Arc<Mutex<AudioStatus>>,
    }

    impl AudioInit {
        /// Spawn the background thread that opens the default output device.
        pub fn spawn() -> AudioInit {
            let out: Arc<Mutex<Option<AudioOut>>> = Arc::new(Mutex::new(None));
            let status: Arc<Mutex<AudioStatus>> =
                Arc::new(Mutex::new(AudioStatus::Starting));
            let init = AudioInit {
                out: out.clone(),
                status: status.clone(),
            };
            let spawned = std::thread::Builder::new()
                .name("audio-init".to_string())
                .spawn(move || {
                    let disabled = std::env::var("MRPC_NO_AUDIO")
                        .map(|v| !v.is_empty() && v != "0")
                        .unwrap_or(false);
                    if disabled {
                        log::info!("[audio] disabled via MRPC_NO_AUDIO");
                        *status.lock().unwrap() =
                            AudioStatus::Failed("disabled (MRPC_NO_AUDIO)".to_string());
                        return;
                    }
                    log::info!("[audio] opening the default output device…");
                    match AudioOut::try_start() {
                        Ok(audio) => {
                            log::info!("[audio] output ready");
                            *out.lock().unwrap() = Some(audio);
                            *status.lock().unwrap() = AudioStatus::Ready;
                        }
                        Err(err) => {
                            log::warn!("[audio] unavailable: {err}");
                            *status.lock().unwrap() = AudioStatus::Failed(err);
                        }
                    }
                });
            if let Err(err) = spawned {
                log::warn!("[audio] could not spawn the init thread: {err}");
                *init.status.lock().unwrap() = AudioStatus::Failed(err.to_string());
            }
            init
        }

        /// Current startup state (polled by the UI).
        pub fn status(&self) -> AudioStatus {
            self.status.lock().unwrap().clone()
        }

        pub fn set_playing(&self, playing: bool) {
            if let Some(audio) = self.out.lock().unwrap().as_mut() {
                audio.set_playing(playing);
            }
        }

        pub fn update(&self, drive: AudioDrive) {
            if let Some(audio) = self.out.lock().unwrap().as_mut() {
                audio.update(drive);
            }
        }

        /// Install the BVE playback engine used by [`AudioDrive::Bve`].
        pub fn set_bve_engine(&self, engine: BvePlaybackEngine) {
            if let Some(audio) = self.out.lock().unwrap().as_mut() {
                audio.set_bve_engine(engine);
            }
        }
    }
}

#[cfg(feature = "audio")]
pub use imp::{AudioDrive, AudioInit, AudioStatus};

#[cfg(not(feature = "audio"))]
mod imp {
    /// Placeholder so the rest of the code can compile without the feature.
    #[allow(dead_code)]
    pub struct AudioOutDisabled;
}

#[cfg(not(feature = "audio"))]
#[allow(dead_code)]
pub type AudioOut = imp::AudioOutDisabled;
