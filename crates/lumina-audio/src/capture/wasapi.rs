//! WASAPI loopback and microphone capture on Windows.
//!
//! # Loopback
//!
//! WASAPI shared mode loopback (`AUDCLNT_STREAMFLAGS_LOOPBACK` on a render
//! endpoint) hands the process the same mix format the desktop mixer is
//! playing, including any userspace or per app volume the user set. That
//! is exactly what the island's visualizer wants: react to what the user
//! hears, not to what a stream nominally contains.
//!
//! # Threading and COM
//!
//! COM objects are apartment bound. Every WASAPI object is created on the
//! worker thread, which first joins the process MTA with
//! `COINIT_MULTITHREADED`, so no interface pointer ever crosses a thread
//! boundary. Open failures travel over the one shot ready channel instead
//! of crossing.
//!
//! # The loop
//!
//! Packet driven: `GetNextPacketSize`, `GetBuffer`, decode to f32, clean
//! via [`CapturePost`](crate::dsp::CapturePost), push into the ring,
//! `ReleaseBuffer`. Between packets a 2 ms sleep keeps the CPU quiet; the
//! hard deadline contract applies to pro audio callbacks, not to a
//! visualizer whose ring drops samples rather than blocking.

use super::{CaptureError, CaptureFormat, CaptureRequest, ReadySignal, spawn_worker, stop_flagged};
use crate::ring::RingWriter;
use std::sync::Arc;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::{
    DEVICE_STATE_ACTIVE, IAudioCaptureClient, IAudioClient, IMMDevice, IMMDeviceCollection,
    IMMDeviceEnumerator, MMDeviceEnumerator, WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
    AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_SHAREMODE_SHARED, eCapture, eConsole, eRender,
};
use windows::Win32::Media::KernelStreaming::WAVE_FORMAT_EXTENSIBLE;
use windows::Win32::Media::Multimedia::{KSDATAFORMAT_SUBTYPE_IEEE_FLOAT, WAVE_FORMAT_IEEE_FLOAT};
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToStringAlloc};
use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ};
use windows::Win32::System::Threading::Sleep;
use windows::core::{Interface, PWSTR, PROPVARIANT};

/// One millisecond in WASAPI reference time: 100 ns ticks.
const REFERENCE_TIME_MS: i64 = 10_000;
/// 20 ms of shared buffer: roomy enough that a busy desktop never starves
/// the ring, shallow enough that the visualizer stays current.
const BUFFER_REFERENCE_TIME: i64 = 20 * REFERENCE_TIME_MS;
/// `AUDCLNT_BUFFERFLAGS_SILENT`: the packet is flagged silent and its
/// contents are undefined, so they must be replaced with zeros.
const AUDCLNT_BUFFERFLAGS_SILENT: u32 = 0x0000_0040;
/// Sleep granularity between packet polls, in ms.
const PACKET_WAIT_MS: u32 = 2;
/// The largest interleaved burst converted per wake: 2048 frames of a
/// normal 2 channel mix is 8 KB of float work, cache friendly.
const BURST_FRAMES: usize = 2048;

/// Sample layout variants the mix format may declare.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SampleLayout {
    /// IEEE float, the usual desktop mix format.
    Float32,
    /// PCM 16 bit, converted into f32 on the way through.
    Int16,
}

/// One endpoint prepared for streaming.
struct ConfiguredStream {
    /// Controls the stream lifetime (`Start`/`Stop`).
    client: IAudioClient,
    /// Pulls packets out of the mix.
    capture: IAudioCaptureClient,
    /// How the mix buffer's samples decode.
    layout: SampleLayout,
    /// Endpoint channel count.
    channels: u16,
    /// Endpoint sample rate.
    sample_rate: u32,
}

/// Spawn a WASAPI worker; the endpoint is resolved on the worker thread so
/// that every COM object stays on the thread that created it.
pub fn spawn(
    request: &CaptureRequest,
    writer: RingWriter,
) -> Result<super::RunningCapture, CaptureError> {
    let source = request.source;
    let device = request.device.clone();
    spawn_worker("wasapi", move |stop: Arc<_>, signal: ReadySignal| {
        run_worker(stop, signal, source, &device, writer);
    })
}

/// Pick the endpoint: the default one for `default`, else a case
/// insensitive substring match against device ids and friendly names.
///
/// # Safety
///
/// Only forwards pointers owned by the COM enumerator to its own methods,
/// and consumes the `PWSTR` the API allocates.
unsafe fn select_device(
    enumerator: &IMMDeviceEnumerator,
    source: lumina_config::schema::AudioSource,
    device: &str,
) -> Result<IMMDevice, CaptureError> {
    let (data_flow, role) = match source {
        lumina_config::schema::AudioSource::System => (eRender, eConsole),
        lumina_config::schema::AudioSource::Microphone => (eCapture, eConsole),
        lumina_config::schema::AudioSource::None => {
            return Err(CaptureError::DeviceUnavailable(
                "audio capture disabled by configuration".into(),
            ));
        }
    };

    if device_is_default(device) {
        return enumerator
            .GetDefaultAudioEndpoint(data_flow, role)
            .map_err(|e| CaptureError::DeviceUnavailable(format!("default endpoint: {e}")));
    }

    let collection: IMMDeviceCollection = enumerator
        .EnumAudioEndpoints(data_flow, DEVICE_STATE_ACTIVE)
        .map_err(|e| CaptureError::DeviceUnavailable(format!("enum endpoints: {e}")))?;
    let count = collection
        .GetCount()
        .map_err(|e| CaptureError::DeviceUnavailable(format!("endpoint count: {e}")))?;

    let wanted = device.trim().to_lowercase();
    for index in 0..count {
        let candidate = match collection.Item(index) {
            Ok(candidate) => candidate,
            Err(_) => continue,
        };
        // One id read requires one free; matching is string only.
        if let Ok(id) = candidate.GetId() {
            let id_text = pwstr_free_to_string(id);
            if id_text.to_lowercase().contains(&wanted) {
                return Ok(candidate);
            }
        }
        if device_friendly_name(&candidate)
            .map_or(false, |name| name.to_lowercase().contains(&wanted))
        {
            return Ok(candidate);
        }
    }
    Err(CaptureError::NoNamedDevice(device.to_string()))
}

/// The friendly name through the property store; absence is a normal
/// outcome, not an error.
///
/// # Safety
///
/// Forwards COM pointers owned by the device object, and frees every
/// allocation the property APIs hand back: the string twice (its `PWSTR`
/// and the `PROPVARIANT` clearing) is exactly once each.
unsafe fn device_friendly_name(device: &IMMDevice) -> Option<String> {
    let properties = device.OpenPropertyStore(STGM_READ).ok()?;
    let prop: PROPVARIANT = properties.GetValue(&PKEY_Device_FriendlyName).ok()?;
    if prop.is_empty() {
        // Nothing registered: there is just no friendly name.
        return None;
    }
    let outcome = PropVariantToStringAlloc(&prop)
        .ok()
        .map(|pwstr| pwstr_free_to_string(pwstr));
    // Clear a deep copy rather than rviolating the caller's reference:
    // `PROPVARIANT`'s `Clone` runs `PropVariantCopy`, so the cleared value
    // is private and the property's own allocation stays owned by `prop`.
    let mut owned = propvariant_take(&prop);
    let _ = PropVariantClear(&mut owned);
    outcome
}

/// A deep copied `PROPVARIANT` ready to hand to [`PropVariantClear`].
///
/// `PropVariantCopy` deep copies any heap attachment, so the cleared
/// value is private and nothing owns the buffer twice.
fn propvariant_take(value: &PROPVARIANT) -> PROPVARIANT {
    value.clone()
}

/// Whether the config's device is the "default" placeholder.
fn device_is_default(device: &str) -> bool {
    device.trim().is_empty() || device.trim().eq_ignore_ascii_case("default")
}

/// Read a `PWSTR` into an owned string, then free its buffer.
///
/// `GetId` and `PropVariantToStringAlloc` both allocate with
/// `CoTaskMemAlloc`; the returned strings are therefore ours to free.
///
/// # Safety
///
/// `pointer` must be a null terminated buffer from a COM API that expects
/// `CoTaskMemFree` (the two calls above). Anything else is undefined.
unsafe fn pwstr_free_to_string(pointer: PWSTR) -> String {
    let text = pointer.to_string().unwrap_or_default();
    windows::Win32::System::Com::CoTaskMemFree(Some(
        pointer.as_ptr().cast::<core::ffi::c_void>(),
    ));
    text
}

/// Parse the mix format pointer the API handed over.
///
/// # Panics
///
/// None. Tag checks happen before any larger struct is dereferenced, and
/// the extensible branch is additionally guarded by `cbSize`.
///
/// # Safety
///
/// `format` must be the allocation returned by `GetMixFormat`, still valid.
unsafe fn parse_mix_format(
    format: *const WAVEFORMATEX,
) -> Result<(SampleLayout, u16, u32), CaptureError> {
    // Windows bindings pack these structs, so every load is an explicit
    // unaligned read instead of a field borrow. Fields are copied to plain
    // locals once, because even a `format!` reference into a packed local
    // is a compile error.
    let base: WAVEFORMATEX = std::ptr::read_unaligned(format);
    let channels = base.nChannels;
    let rate = base.nSamplesPerSec;
    let bits = base.wBitsPerSample;
    let tag = base.wFormatTag;

    if tag == WAVE_FORMAT_IEEE_FLOAT as u16 {
        return Ok((SampleLayout::Float32, channels, rate));
    }
    if tag == WAVE_FORMAT_EXTENSIBLE as u16 {
        // WAVEFORMATEXTENSIBLE extends WAVEFORMATEX by 22 bytes. The API
        // may hand a truncated buffer, so the byte budget is verified
        // before the struct is read.
        let extension = std::mem::size_of::<WAVEFORMATEXTENSIBLE>()
            - std::mem::size_of::<WAVEFORMATEX>();
        if base.cbSize < extension as u16 {
            return Err(CaptureError::DeviceUnavailable(
                "mix format marks extensible but the struct is truncated".into(),
            ));
        }
        let extensible: WAVEFORMATEXTENSIBLE =
            std::ptr::read_unaligned(format.cast::<WAVEFORMATEXTENSIBLE>());
        let sub_format = extensible.SubFormat;
        if sub_format == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT {
            return Ok((SampleLayout::Float32, channels, rate));
        }
        if bits == 16 {
            // Bit packed 16 in an extensible bait: still PCM 16.
            return Ok((SampleLayout::Int16, channels, rate));
        }
        return Err(CaptureError::DeviceUnavailable(format!(
            "unsupported extensible subformat at {bits} bit: only f32 and i16 are handled"
        )));
    }
    if tag == 1 && bits == 16 {
        // Legacy PCM 16.
        return Ok((SampleLayout::Int16, channels, rate));
    }
    Err(CaptureError::DeviceUnavailable(format!(
        "unsupported mix format (tag {tag}, {bits} bit)"
    )))
}

/// The worker body: COM init, endpoint resolution, then the pull loop.
fn run_worker(
    stop: Arc<std::sync::atomic::AtomicBool>,
    signal: ReadySignal,
    source: lumina_config::schema::AudioSource,
    device: &str,
    writer: RingWriter,
) {
    // COM apartment setup is per thread. On a joining thread an HRESULT
    // of S_FALSE ("already a member") is a normal outcome; keep it.
    unsafe {
        if let Err(e) = com_init_thread(None, COINIT_MULTITHREADED) {
            signal.report(Err(CaptureError::StreamError(format!("CoInitializeEx: {e}"))));
            return;
        }
    }

    let endpoint = unsafe { open_endpoint(source, device) };
    let endpoint = match endpoint {
        Ok(endpoint) => endpoint,
        Err(error) => {
            signal.report(Err(error));
            return;
        }
    };

    let format = CaptureFormat {
        sample_rate: endpoint.sample_rate,
        channels: endpoint.channels,
        loopback: matches!(source, lumina_config::schema::AudioSource::System),
    };
    signal.report(Ok(format));
    // One shot sent; later failures are handled inside the loop.
    drop(signal);

    let mut post = crate::dsp::CapturePost::new(endpoint.channels, endpoint.sample_rate);
    let mut interleaved = vec![0.0f32; BURST_FRAMES * endpoint.channels as usize];
    let mut raw16 = vec![0i16; BURST_FRAMES * endpoint.channels as usize];
    let mut mono_scratch = vec![0.0f32; BURST_FRAMES];

    unsafe {
        loop {
            if stop_flagged(&stop) {
                // Flush the endpoint state; the ring simply keeps whatever
                // samples were pushed.
                let _ = endpoint.client.Stop();
                return;
            }

            // Query one packet at a time; a zero packet count means the
            // mixer has nothing new this instant, so rest briefly.
            let packet_frames = match endpoint.capture.GetNextPacketSize() {
                Ok(frames) => frames,
                Err(_) => {
                    Sleep(PACKET_WAIT_MS);
                    continue;
                }
            };
            if packet_frames == 0 {
                Sleep(PACKET_WAIT_MS);
                continue;
            }

            let mut data: *mut u8 = std::ptr::null_mut();
            let mut readable: u32 = 0;
            let mut flags: u32 = 0;
            if endpoint
                .capture
                .GetBuffer(&mut data, &mut readable, &mut flags, None, None)
                .is_err()
            {
                Sleep(PACKET_WAIT_MS);
                continue;
            }

            push_packet(
                &mut post,
                &writer,
                &endpoint,
                data,
                readable,
                flags,
                &mut interleaved,
                &mut raw16,
                &mut mono_scratch,
            );

            let _ = endpoint.capture.ReleaseBuffer(readable);
        }
    }
}

/// Convert one returned packet and push it through the ring.
///
/// # Safety
///
/// `data` must point to `readable * channels` samples of the layout in
/// [`ConfiguredStream`], exactly as handed out by `GetBuffer`.
#[allow(clippy::too_many_arguments)]
unsafe fn push_packet(
    post: &mut crate::dsp::CapturePost,
    writer: &RingWriter,
    endpoint: &ConfiguredStream,
    data: *mut u8,
    readable: u32,
    flags: u32,
    interleaved: &mut [f32],
    raw16: &mut [i16],
    mono_scratch: &mut [f32],
) {
    let channels = endpoint.channels as usize;
    let frames = (readable as usize).min(BURST_FRAMES).min(mono_scratch.len());
    if frames == 0 {
        return;
    }
    let count = frames * channels;
    let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT != 0;

    match endpoint.layout {
        SampleLayout::Float32 => {
            if silent {
                interleaved[..count].fill(0.0);
            } else {
                // SAFETY: GetBuffer hands `readable * channels` f32s of
                // mix data; `count` covers that exactly.
                let mix = std::slice::from_raw_parts(data.cast::<f32>(), count);
                interleaved[..count].copy_from_slice(mix);
            }
        }
        SampleLayout::Int16 => {
            if silent {
                raw16[..count].fill(0);
            } else {
                // SAFETY: same packet shape as above, in i16 units.
                let mix = std::slice::from_raw_parts(data.cast::<i16>(), count);
                raw16[..count].copy_from_slice(mix);
            }
            for (float_slot, &pcm) in interleaved[..count]
                .iter_mut()
                .zip(raw16[..count].iter())
            {
                *float_slot = f32::from(pcm) / 32_768.0;
            }
        }
    }

    let produced = post.process_interleaved(&interleaved[..count], &mut mono_scratch[..frames]);
    if produced > 0 {
        writer.write(&mono_scratch[..produced]);
    }
}

/// Open and start the endpoint.
///
/// # Safety
///
/// Every COM object is created here on the calling thread and stays there;
/// allocations from the API are freed on all paths, success and failure.
unsafe fn open_endpoint(
    source: lumina_config::schema::AudioSource,
    device: &str,
) -> Result<ConfiguredStream, CaptureError> {
    let enumerator: IMMDeviceEnumerator =
        com_create_object(&MMDeviceEnumerator, CLSCTX_ALL)
            .map_err(|e| CaptureError::DeviceUnavailable(format!("COM enumerator: {e}")))?;

    let endpoint: IMMDevice = select_device(&enumerator, source, device)?;

    // Activate is the typed wrapper in windows-rs 0.58: interface and iid
    // resolved by the generic, activation params unused for audio clients.
    let client: IAudioClient =
        endpoint
            .Activate::<IAudioClient>(CLSCTX_ALL, None)
            .map_err(|e| CaptureError::DeviceUnavailable(format!("IAudioClient activate: {e}")))?;

    let mix_fmt = client
        .GetMixFormat()
        .map_err(|e| CaptureError::DeviceUnavailable(format!("GetMixFormat: {e}")))?;
    if mix_fmt.is_null() {
        com_free(Some(mix_fmt.cast::<core::ffi::c_void>()));
        return Err(CaptureError::DeviceUnavailable(
            "GetMixFormat returned a null format".into(),
        ));
    }
    let parsed = parse_mix_format(mix_fmt);
    let (layout, channels, sample_rate) = match parsed {
        Ok(parsed) => parsed,
        Err(error) => {
            com_free(Some(mix_fmt.cast::<core::ffi::c_void>()));
            return Err(error);
        }
    };

    let loopback = matches!(source, lumina_config::schema::AudioSource::System);
    let flags: u32 = if loopback {
        AUDCLNT_STREAMFLAGS_LOOPBACK
    } else {
        0
    };

    if let Err(e) = client.Initialize(
        AUDCLNT_SHAREMODE_SHARED,
        flags,
        BUFFER_REFERENCE_TIME,
        0,
        mix_fmt,
        None,
    ) {
        com_free(Some(mix_fmt.cast::<core::ffi::c_void>()));
        return Err(CaptureError::StreamError(format!("IAudioClient::Initialize: {e}")));
    }
    // The format was a loan: the client copies what Initialize needs.
    com_free(Some(mix_fmt.cast::<core::ffi::c_void>()));

    let capture: IAudioCaptureClient = client
        .GetService()
        .map_err(|e| CaptureError::StreamError(format!("GetService(IAudioCaptureClient): {e}")))?;

    client
        .Start()
        .map_err(|e| CaptureError::StreamError(format!("IAudioClient::Start: {e}")))?;

    Ok(ConfiguredStream {
        client,
        capture,
        layout,
        channels,
        sample_rate,
    })
}

/// Thin local wrappers keeping the awkward generic/Option call shapes out
/// of the body of the flow functions.

/// `CoCreateInstance` for apartment aware class ids, typed directly.
///
/// # Safety
///
/// Calls into the COM type system; `T` must implement the queried mode.
unsafe fn com_create_object<T: Interface>(
    clsid: &windows::core::GUID,
    context: windows::Win32::System::Com::CLSCTX,
) -> windows::core::Result<T> {
    windows::Win32::System::Com::CoCreateInstance(clsid, None, context)
}

/// `CoTaskMemFree` with the `Option` shape the binding wants.
///
/// # Safety
///
/// `pointer` must be a live `CoTaskMemAlloc` allocation or `None`.
unsafe fn com_free(pointer: Option<*const core::ffi::c_void>) {
    windows::Win32::System::Com::CoTaskMemFree(pointer);
}

/// `CoInitializeEx` mapped from `HRESULT` to `Result`.
///
/// # Safety
///
/// Standard COM thread init; must be matched by the thread's lifetime.
unsafe fn com_init_thread(
    reserved: Option<*const core::ffi::c_void>,
    init: windows::Win32::System::Com::COINIT,
) -> windows::core::Result<()> {
    windows::Win32::System::Com::CoInitializeEx(reserved, init).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mix_format_recognizes_float_rectifies_pcm_and_refuses_unknowns() {
        let float = WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_IEEE_FLOAT as u16,
            nChannels: 2,
            nSamplesPerSec: 48_000,
            nAvgBytesPerSec: 48_000 * 2 * 4,
            nBlockAlign: 8,
            wBitsPerSample: 32,
            cbSize: 0,
        };
        let parsed = unsafe { parse_mix_format(&float) };
        assert_eq!(parsed.expect("float parses"), (SampleLayout::Float32, 2, 48_000));

        let pcm = WAVEFORMATEX {
            wFormatTag: 1,
            nChannels: 2,
            nSamplesPerSec: 44_100,
            nAvgBytesPerSec: 44_100 * 2 * 2,
            nBlockAlign: 4,
            wBitsPerSample: 16,
            cbSize: 0,
        };
        let parsed = unsafe { parse_mix_format(&pcm) };
        assert_eq!(parsed.expect("pcm parses"), (SampleLayout::Int16, 2, 44_100));

        let junk = WAVEFORMATEX {
            wFormatTag: 0x4242,
            nChannels: 2,
            nSamplesPerSec: 48_000,
            nAvgBytesPerSec: 48_000 * 2 * 3,
            nBlockAlign: 6,
            wBitsPerSample: 24,
            cbSize: 0,
        };
        match unsafe { parse_mix_format(&junk) } {
            Err(CaptureError::DeviceUnavailable(msg)) => {
                assert!(msg.contains("unsupported mix format"), "message: {msg}");
            }
            other => panic!("expected refusal, got {other:?}"),
        }
    }

    #[test]
    fn default_device_detection_is_tolerant() {
        assert!(device_is_default("default"));
        assert!(device_is_default(" DEFAULT "));
        assert!(device_is_default(""));
        assert!(!device_is_default("Speakers (Realtek)"));
    }

    #[test]
    fn propvariant_take_never_leaks_the_value() {
        // An empty variant clone/clear round trip is a compile of the
        // ownership helpers without hardware.
        let value = PROPVARIANT::new();
        assert!(value.is_empty());
        let taken = propvariant_take(&value);
        assert!(taken.is_empty());
    }
}
