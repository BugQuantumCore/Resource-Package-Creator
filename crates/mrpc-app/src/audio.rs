//! Live VVVF audition through the audio output (feature `audio`, cpal).
//!
//! The GUI thread updates [`PlaybackParams`] each frame; the cpal callback
//! pulls samples from the C++ synthesis engine.

#[cfg(feature = "audio")]
mod imp {
    use std::sync::{Arc, Mutex};

    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    use mrpc_vvvf::{Engine, VvvfParams};

    #[derive(Clone, Copy)]
    pub struct PlaybackParams {
        pub params: VvvfParams,
        pub fundamental_hz: f64,
        pub power: f64,
        pub playing: bool,
    }

    struct Shared {
        state: Mutex<PlaybackParams>,
    }

    pub struct AudioOut {
        shared: Arc<Shared>,
        _stream: cpal::Stream,
        sample_rate: f64,
    }

    impl AudioOut {
        pub fn try_start() -> Result<AudioOut, String> {
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
                    params: VvvfParams {
                        sample_rate,
                        ..Default::default()
                    },
                    fundamental_hz: 0.0,
                    power: 0.0,
                    playing: false,
                }),
            });

            let shared_cb = shared.clone();
            let err_fn = |err| eprintln!("[audio] {err}");
            // cpal 0.18 takes the stream config by value.
            let stream = device
                .build_output_stream(
                    stream_config,
                    move |data: &mut [f32], _info: &cpal::OutputCallbackInfo| {
                        let snapshot = {
                            let guard = shared_cb.state.lock().unwrap();
                            *guard
                        };
                        if !snapshot.playing {
                            for d in data.iter_mut() {
                                *d = 0.0;
                            }
                            return;
                        }
                        // One engine per callback keeps phase continuity per
                        // block cheap; acceptable for a preview tool.
                        let mut engine = Engine::new(snapshot.params);
                        let mut mono = vec![0f32; data.len() / channels.max(1)];
                        engine.render(snapshot.fundamental_hz, snapshot.power, &mut mono);
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

            Ok(AudioOut { shared, _stream: stream, sample_rate })
        }

        pub fn set_playing(&mut self, playing: bool) {
            self.shared.state.lock().unwrap().playing = playing;
        }

        pub fn update(&mut self, mut params: VvvfParams, fundamental_hz: f64, power: f64) {
            params.sample_rate = self.sample_rate;
            let mut guard = self.shared.state.lock().unwrap();
            guard.params = params;
            guard.fundamental_hz = fundamental_hz;
            guard.power = power;
        }
    }
}

#[cfg(feature = "audio")]
pub use imp::AudioOut;

#[cfg(not(feature = "audio"))]
mod imp {
    /// Placeholder so the rest of the code can compile without the feature.
    #[allow(dead_code)]
    pub struct AudioOutDisabled;
}

#[cfg(not(feature = "audio"))]
#[allow(dead_code)]
pub type AudioOut = imp::AudioOutDisabled;
