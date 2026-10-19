# lumina-audio

Audio capture and spectrum analysis for [Lumina](https://github.com/lumina-sh/lumina),
the dynamic island overlay for Windows, X11, and Wayland.

The crate turns whatever the system is playing into 32 normalized band
magnitudes that a 60 fps visualizer can draw, within a CPU budget of roughly
0.22 % of one core on a 2019 class laptop.

## Pipeline

```text
capture callback (stereo f32, 20 ms period)
  -> downmix to mono                    (dsp::Downmixer)
  -> DC removal, alpha = 0.999          (dsp::DcRemover)
  -> one pole highpass at 30 Hz         (dsp::HighPass)
  -> SPSC ring, 8192 samples            (ring::RingBuffer)
  -------------------------------------------  (analysis thread, ~47 Hz)
  -> read 4096 samples                  (ring::RingReader)
  -> Hann window, packed real FFT       (fft::Analyzer)
  -> log band mapping, 32 bands, dB     (bands::BandMapper)
  -> bass tilt, +3 dB/octave < 200 Hz
  -> envelope follower a=0.35 r=0.08    (envelope::BandEnvelope)
  -> silence gate + rolling normalize   (dynamics)
  -> SpectrumFrame, polled at 60 Hz     (engine::AudioEngine)
```

## Guarantees

1. **The capture callback never allocates, never locks, and never blocks.**
   When the ring is full the newest samples are dropped, because a blocked
   callback clicks in the user's music and a dropped sample is invisible.
2. **No allocation in the analysis path after warmup.** Windows, bin edges,
   buffers, and the FFT plan are built once.
3. **Silence looks silent.** A -100 dBFS input produces zero magnitudes within
   200 ms, never a jittering noise floor.
4. **Unplugging is not a crash.** A lost device latches to
   [`AudioState::Unavailable`] and emits a single warning, per the PLAN §9
   modality matrix.

## Platform support

| Platform | Backend | Source |
|---|---|---|
| Windows | WASAPI loopback (`capture::wasapi`) | default render endpoint |
| Linux, PulseAudio | `pulse` (`capture::pulse`) | default sink monitor |
| Linux, PipeWire | `pipewire` (`capture::pipewire`) | pulse-compatible monitor |
| Linux, neither | none | spectrum renders `audio unavailable` |

PipeWire capture goes through PipeWire's own PulseAudio compatibility server
(`module-protocol-pulse`), which every mainstream distribution enables by
default. Linking `libpipewire` directly against its private protocol would pin
the overlay to one PipeWire version and break at every ABI bump; going through
the stable Pulse protocol surface that PipeWire itself maintains is the
portable choice.

## Testing without audio hardware

Everything above the capture layer is hardware independent. Tests drive the
pipeline with synthetic signals: a 1 kHz sine, pink-ish noise shaped to look
like music, amplitude-modulated speech-like audio, and digital silence. The
threaded capture backends are exercised only for their lifecycle logic; real
device I/O needs a desktop session.
