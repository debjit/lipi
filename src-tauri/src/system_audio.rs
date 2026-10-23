use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

pub struct PlaybackCapture {
    pub samples: Arc<Mutex<Vec<f32>>>,
    pub sample_rate: u32,
    pub channels: u16,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for PlaybackCapture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

pub fn open_playback() -> Result<PlaybackCapture, String> {
    #[cfg(windows)]
    {
        return windows::open();
    }
    #[cfg(target_os = "linux")]
    {
        return linux::open();
    }
    #[cfg(target_os = "macos")]
    {
        return macos::open();
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        Err("System audio capture is not available on this system.".into())
    }
}

#[cfg(windows)]
mod windows {
    use super::PlaybackCapture;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;
    use wasapi::{
        DeviceEnumerator, Direction, SampleType, StreamMode, WasapiError, initialize_mta,
    };

    pub fn open() -> Result<PlaybackCapture, String> {
        let samples = Arc::new(Mutex::new(Vec::<f32>::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let samples_thread = Arc::clone(&samples);
        let stop_thread = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            match setup() {
                Ok(capture) => {
                    let _ = ready_tx.send(Ok((capture.sample_rate, capture.channels)));
                    poll_loop(capture, samples_thread, stop_thread);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            }
        });
        match ready_rx.recv() {
            Ok(Ok((sample_rate, channels))) => Ok(PlaybackCapture {
                samples,
                sample_rate,
                channels,
                stop,
                thread: Some(handle),
            }),
            Ok(Err(e)) => {
                let _ = handle.join();
                Err(e)
            }
            Err(_) => {
                let _ = handle.join();
                Err("System audio capture stopped before it opened.".into())
            }
        }
    }

    struct Loopback {
        client: wasapi::AudioClient,
        capture: wasapi::AudioCaptureClient,
        sample_rate: u32,
        channels: u16,
        bits: u16,
        kind: SampleType,
        block: usize,
    }

    fn setup() -> Result<Loopback, String> {
        let com = initialize_mta();
        if com.is_err() {
            return Err(format!("System audio did not open: COM initialization failed ({com})."));
        }
        let enumerator = DeviceEnumerator::new().map_err(wasapi_err)?;
        let device = enumerator
            .get_default_device(&Direction::Render)
            .map_err(wasapi_err)?;
        let mut client = device.get_iaudioclient().map_err(wasapi_err)?;
        let format = client.get_mixformat().map_err(wasapi_err)?;
        let kind = format.get_subformat().map_err(wasapi_err)?;
        let bits = format.get_bitspersample();
        if !supported_format(kind, bits) {
            return Err(format!(
                "System audio format is not supported ({kind:?}, {bits}-bit)."
            ));
        }
        let (default_period, _) = client.get_device_period().map_err(wasapi_err)?;
        let buffer_duration_hns = if default_period > 0 { default_period } else { 200_000 };
        let mode = StreamMode::PollingShared {
            autoconvert: true,
            buffer_duration_hns,
        };
        client
            .initialize_client(&format, &Direction::Capture, &mode)
            .map_err(wasapi_err)?;
        let capture = client.get_audiocaptureclient().map_err(wasapi_err)?;
        client.start_stream().map_err(wasapi_err)?;
        Ok(Loopback {
            client,
            capture,
            sample_rate: format.get_samplespersec().max(1),
            channels: format.get_nchannels().max(1),
            bits,
            kind,
            block: format.get_blockalign().max(1) as usize,
        })
    }

    fn supported_format(kind: SampleType, bits: u16) -> bool {
        matches!(
            (kind, bits),
            (SampleType::Float, 32) | (SampleType::Int, 16) | (SampleType::Int, 32)
        )
    }

    fn poll_loop(loopback: Loopback, samples: Arc<Mutex<Vec<f32>>>, stop: Arc<AtomicBool>) {
        let Loopback {
            client,
            capture,
            channels,
            bits,
            kind,
            block,
            ..
        } = loopback;
        let mut byte_buf = vec![0u8; block.saturating_mul(48_000).max(block)];
        while !stop.load(Ordering::SeqCst) {
            let mut read_any = false;
            loop {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                let frames = match capture.get_next_packet_size() {
                    Ok(Some(frames)) => frames,
                    Ok(None) => 0,
                    Err(_) => break,
                };
                if frames == 0 {
                    break;
                }
                let need = frames as usize * block;
                if byte_buf.len() < need {
                    byte_buf.resize(need, 0);
                }
                let read = match capture.read_from_device(&mut byte_buf[..need]) {
                    Ok((read, info)) => {
                        if info.flags.silent {
                            push_zeros(&samples, read as usize * channels as usize);
                            read_any = true;
                            continue;
                        }
                        read
                    }
                    Err(_) => break,
                };
                if read == 0 {
                    break;
                }
                let pcm = decode_frames(
                    &byte_buf[..read as usize * block],
                    read as usize,
                    channels,
                    bits,
                    kind,
                );
                push_samples(&samples, &pcm);
                read_any = true;
            }
            if !read_any {
                thread::sleep(Duration::from_millis(5));
            }
        }
        let _ = client.stop_stream();
    }

    fn decode_frames(bytes: &[u8], frames: usize, channels: u16, bits: u16, kind: SampleType) -> Vec<f32> {
        let count = frames.saturating_mul(channels.max(1) as usize);
        let mut out = Vec::with_capacity(count);
        match (kind, bits) {
            (SampleType::Float, 32) => {
                for chunk in bytes.chunks_exact(4).take(count) {
                    out.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
                }
            }
            (SampleType::Int, 16) => {
                for chunk in bytes.chunks_exact(2).take(count) {
                    let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
                    out.push(sample as f32 / 32768.0);
                }
            }
            (SampleType::Int, 32) => {
                for chunk in bytes.chunks_exact(4).take(count) {
                    let sample = i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                    out.push(sample as f32 / 2147483648.0);
                }
            }
            _ => {}
        }
        out
    }

    fn push_samples(samples: &Mutex<Vec<f32>>, pcm: &[f32]) {
        if let Ok(mut lock) = samples.lock() {
            lock.extend_from_slice(pcm);
        }
    }

    fn push_zeros(samples: &Mutex<Vec<f32>>, count: usize) {
        if count == 0 {
            return;
        }
        if let Ok(mut lock) = samples.lock() {
            let new_len = lock.len() + count;
            lock.resize(new_len, 0.0);
        }
    }

    fn wasapi_err(err: WasapiError) -> String {
        format!("System audio did not open: {err}")
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::PlaybackCapture;
    use libpulse_binding::context::{Context, FlagSet as ContextFlags, State as ContextState};
    use libpulse_binding::mainloop::standard::{IterateResult, Mainloop};
    use libpulse_binding::sample::{Format, Spec};
    use libpulse_binding::stream::{FlagSet as StreamFlags, PeekResult, State as StreamState, Stream};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    const RATE: u32 = 48_000;
    const CHANNELS: u16 = 2;

    pub fn open() -> Result<PlaybackCapture, String> {
        let samples = Arc::new(Mutex::new(Vec::<f32>::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let samples_thread = Arc::clone(&samples);
        let stop_thread = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            if let Err(err) = record(ready_tx, samples_thread, stop_thread) {
                eprintln!("System audio capture ended: {err}");
            }
        });
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(PlaybackCapture {
                samples,
                sample_rate: RATE,
                channels: CHANNELS,
                stop,
                thread: Some(handle),
            }),
            Ok(Err(err)) => {
                stop.store(true, Ordering::SeqCst);
                let _ = handle.join();
                Err(err)
            }
            Err(_) => {
                stop.store(true, Ordering::SeqCst);
                let _ = handle.join();
                Err("System audio capture stopped before it opened.".into())
            }
        }
    }

    struct Ready {
        tx: Option<std::sync::mpsc::Sender<Result<(), String>>>,
    }

    impl Ready {
        fn new(tx: std::sync::mpsc::Sender<Result<(), String>>) -> Self {
            Self { tx: Some(tx) }
        }

        fn send(&mut self, result: Result<(), String>) {
            if let Some(tx) = self.tx.take() {
                let _ = tx.send(result);
            }
        }
    }

    fn record(
        ready_tx: std::sync::mpsc::Sender<Result<(), String>>,
        samples: Arc<Mutex<Vec<f32>>>,
        stop: Arc<AtomicBool>,
    ) -> Result<(), String> {
        let mut ready = Ready::new(ready_tx);
        let mut mainloop = match Mainloop::new() {
            Some(mainloop) => mainloop,
            None => {
                let err = "PulseAudio mainloop failed to start.".to_string();
                ready.send(Err(err.clone()));
                return Err(err);
            }
        };
        let mut context = match Context::new(&mainloop, "Lipi") {
            Some(context) => context,
            None => {
                let err = "PulseAudio context failed to start.".to_string();
                ready.send(Err(err.clone()));
                return Err(err);
            }
        };
        if let Err(err) = context.connect(None, ContextFlags::empty(), None) {
            let err = format!("PulseAudio did not connect: {err}");
            ready.send(Err(err.clone()));
            return Err(err);
        }
        for _ in 0..400 {
            let _ = mainloop.iterate(false);
            match context.get_state() {
                ContextState::Ready => break,
                ContextState::Failed | ContextState::Terminated => {
                    let err = format!("PulseAudio connection failed: {}", context.errno());
                    ready.send(Err(err.clone()));
                    return Err(err);
                }
                _ => thread::sleep(Duration::from_millis(5)),
            }
        }
        if context.get_state() != ContextState::Ready {
            let err = "PulseAudio or PipeWire did not become ready.".to_string();
            ready.send(Err(err.clone()));
            return Err(err);
        }
        let spec = Spec {
            format: Format::FLOAT32NE,
            rate: RATE,
            channels: CHANNELS as u8,
        };
        if !spec.is_valid() {
            let err = "PulseAudio rejected the playback format.".to_string();
            ready.send(Err(err.clone()));
            return Err(err);
        }
        let mut stream = match Stream::new(&mut context, "Meeting playback", &spec, None) {
            Some(stream) => stream,
            None => {
                let err = "PulseAudio record stream failed to start.".to_string();
                ready.send(Err(err.clone()));
                return Err(err);
            }
        };
        if let Err(err) = stream.connect_record(Some("@DEFAULT_MONITOR@"), None, StreamFlags::empty()) {
            let err = format!("Playback monitor did not open: {err}");
            ready.send(Err(err.clone()));
            return Err(err);
        }
        for _ in 0..400 {
            let _ = mainloop.iterate(false);
            match stream.get_state() {
                StreamState::Ready => break,
                StreamState::Failed | StreamState::Terminated => {
                    let err = "Playback monitor did not open. Headphones and webinar audio will not be recorded.".to_string();
                    ready.send(Err(err.clone()));
                    return Err(err);
                }
                _ => thread::sleep(Duration::from_millis(5)),
            }
        }
        if stream.get_state() != StreamState::Ready {
            let err = "Playback monitor did not open.".to_string();
            ready.send(Err(err.clone()));
            return Err(err);
        }
        ready.send(Ok(()));
        while !stop.load(Ordering::SeqCst) {
            match mainloop.iterate(false) {
                IterateResult::Quit(_) | IterateResult::Err(_) => break,
                IterateResult::Success(_) => {}
            }
            match stream.peek() {
                Ok(PeekResult::Data(bytes)) => {
                    let mut pcm = Vec::with_capacity(bytes.len() / 4);
                    for chunk in bytes.chunks_exact(4) {
                        pcm.push(f32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
                    }
                    if let Ok(mut lock) = samples.lock() {
                        lock.extend_from_slice(&pcm);
                    }
                    let _ = stream.discard();
                }
                Ok(PeekResult::Hole(_)) => {
                    let _ = stream.discard();
                }
                Ok(PeekResult::Empty) => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(_) => break,
            }
        }
        stream.disconnect().ok();
        context.disconnect();
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::PlaybackCapture;
    use core_foundation::array::CFArray;
    use core_foundation::base::TCFType;
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::string::CFString;
    use objc2::runtime::AnyClass;
    use objc2::AnyThread;
    use objc2_core_audio::{
        AudioObjectPropertyAddress, CATapDescription, kAudioAggregateDeviceIsPrivateKey,
        kAudioAggregateDeviceNameKey, kAudioAggregateDeviceTapAutoStartKey, kAudioAggregateDeviceTapListKey,
        kAudioAggregateDeviceUIDKey, kAudioDevicePropertyNominalSampleRate, kAudioObjectPropertyElementMain,
        kAudioObjectPropertyScopeGlobal, kAudioSubTapDriftCompensationKey, kAudioSubTapUIDKey,
    };
    use objc2_core_audio_types::{AudioBuffer, AudioBufferList};
    use objc2_foundation::{NSArray, NSNumber, NSString};
    use std::ffi::{CStr, CString, c_char, c_void};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    extern "C" {
        fn dlopen(filename: *const c_char, flags: i32) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        fn dlclose(handle: *mut c_void) -> i32;
    }

    const RTLD_NOW: i32 = 2;

    type IoProc = unsafe extern "C" fn(
        u32,
        *const c_void,
        *const AudioBufferList,
        *const c_void,
        *mut c_void,
        *const c_void,
        *mut c_void,
    ) -> i32;
    type CreateTap = unsafe extern "C" fn(*mut CATapDescription, *mut u32) -> i32;
    type DestroyTap = unsafe extern "C" fn(u32) -> i32;
    type CreateAggregate = unsafe extern "C" fn(*const c_void, *mut u32) -> i32;
    type DestroyAggregate = unsafe extern "C" fn(u32) -> i32;
    type CreateIo = unsafe extern "C" fn(u32, IoProc, *mut c_void, *mut *mut c_void) -> i32;
    type DestroyIo = unsafe extern "C" fn(u32, *mut c_void) -> i32;
    type DeviceStart = unsafe extern "C" fn(u32, *mut c_void) -> i32;
    type DeviceStop = unsafe extern "C" fn(u32, *mut c_void) -> i32;
    type GetProperty = unsafe extern "C" fn(
        u32,
        *const AudioObjectPropertyAddress,
        u32,
        *const c_void,
        *mut u32,
        *mut c_void,
    ) -> i32;

    struct CoreFns {
        create_tap: CreateTap,
        destroy_tap: DestroyTap,
        create_aggregate: CreateAggregate,
        destroy_aggregate: DestroyAggregate,
        create_io: CreateIo,
        destroy_io: DestroyIo,
        start: DeviceStart,
        stop: DeviceStop,
        get_property: GetProperty,
    }

    struct TapShared {
        samples: Arc<Mutex<Vec<f32>>>,
    }

    pub fn open() -> Result<PlaybackCapture, String> {
        if AnyClass::get(c"CATapDescription").is_none() {
            return Err("System audio capture needs macOS 14.2 or later.".into());
        }
        let samples = Arc::new(Mutex::new(Vec::<f32>::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let samples_thread = Arc::clone(&samples);
        let stop_thread = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            if let Err(err) = run(ready_tx, samples_thread, stop_thread) {
                eprintln!("System audio capture ended: {err}");
            }
        });
        match ready_rx.recv() {
            Ok(Ok((sample_rate, channels))) => Ok(PlaybackCapture {
                samples,
                sample_rate,
                channels,
                stop,
                thread: Some(handle),
            }),
            Ok(Err(err)) => {
                stop.store(true, Ordering::SeqCst);
                let _ = handle.join();
                Err(err)
            }
            Err(_) => {
                stop.store(true, Ordering::SeqCst);
                let _ = handle.join();
                Err("System audio capture stopped before it opened.".into())
            }
        }
    }

    fn run(
        ready_tx: std::sync::mpsc::Sender<Result<(u32, u16), String>>,
        samples: Arc<Mutex<Vec<f32>>>,
        stop: Arc<AtomicBool>,
    ) -> Result<(), String> {
        let mut ready_tx = Some(ready_tx);
        let report = |ready_tx: &mut Option<std::sync::mpsc::Sender<Result<(u32, u16), String>>>, res| {
            if let Some(tx) = ready_tx.take() {
                let _ = tx.send(res);
            }
        };
        let lib = unsafe {
            dlopen(
                b"/System/Library/Frameworks/CoreAudio.framework/CoreAudio\0".as_ptr() as *const c_char,
                RTLD_NOW,
            )
        };
        if lib.is_null() {
            let err = "Core Audio could not be opened.".to_string();
            report(&mut ready_tx, Err(err.clone()));
            return Err(err);
        }
        let fns = match load_fns(lib) {
            Ok(fns) => fns,
            Err(err) => {
                unsafe { dlclose(lib) };
                report(&mut ready_tx, Err(err.clone()));
                return Err(err);
            }
        };
        let excluded: &[&NSNumber] = &[];
        let empty = NSArray::from_slice(excluded);
        let desc = unsafe {
            CATapDescription::initStereoGlobalTapButExcludeProcesses(CATapDescription::alloc(), &empty)
        };
        unsafe {
            desc.setPrivate(true);
            desc.setName(&NSString::from_str("Lipi"));
        }
        let mut tap_id: u32 = 0;
        let status = unsafe { (fns.create_tap)(&*desc as *const CATapDescription as *mut CATapDescription, &mut tap_id) };
        if status != 0 {
            unsafe { dlclose(lib) };
            let err = format!("System audio tap did not open ({status}).");
            report(&mut ready_tx, Err(err.clone()));
            return Err(err);
        }
        let tap_uid = format!("{}", &*unsafe { desc.UUID().UUIDString() });
        let aggregate_uid = format!("lipi-{tap_uid}");
        let sub = cf_dict(&[
            (key_string(kAudioSubTapUIDKey), CFString::new(&tap_uid).as_CFType()),
            (
                key_string(kAudioSubTapDriftCompensationKey),
                CFBoolean::true_value().as_CFType(),
            ),
        ]);
        let taps = CFArray::from_CFTypes(&[sub]);
        let aggregate = cf_dict(&[
            (
                key_string(kAudioAggregateDeviceNameKey),
                CFString::new("Lipi System Audio").as_CFType(),
            ),
            (
                key_string(kAudioAggregateDeviceUIDKey),
                CFString::new(&aggregate_uid).as_CFType(),
            ),
            (
                key_string(kAudioAggregateDeviceIsPrivateKey),
                CFBoolean::true_value().as_CFType(),
            ),
            (
                key_string(kAudioAggregateDeviceTapAutoStartKey),
                CFBoolean::true_value().as_CFType(),
            ),
            (
                key_string(kAudioAggregateDeviceTapListKey),
                taps.as_CFType(),
            ),
        ]);
        let mut device_id: u32 = 0;
        let status = unsafe {
            (fns.create_aggregate)(
                aggregate.as_concrete_TypeRef() as *const c_void,
                &mut device_id,
            )
        };
        if status != 0 {
            unsafe { (fns.destroy_tap)(tap_id) };
            unsafe { dlclose(lib) };
            let err = format!("System audio device did not open ({status}).");
            report(&mut ready_tx, Err(err.clone()));
            return Err(err);
        }
        let shared = Box::into_raw(Box::new(TapShared { samples }));
        let mut proc_id: *mut c_void = std::ptr::null_mut();
        let status = unsafe { (fns.create_io)(device_id, tap_io, shared as *mut c_void, &mut proc_id) };
        if status != 0 || proc_id.is_null() {
            unsafe {
                drop(Box::from_raw(shared));
                (fns.destroy_aggregate)(device_id);
                (fns.destroy_tap)(tap_id);
                dlclose(lib);
            }
            let err = format!("System audio callback did not start ({status}).");
            report(&mut ready_tx, Err(err.clone()));
            return Err(err);
        }
        let status = unsafe { (fns.start)(device_id, proc_id) };
        if status != 0 {
            unsafe {
                (fns.destroy_io)(device_id, proc_id);
                drop(Box::from_raw(shared));
                (fns.destroy_aggregate)(device_id);
                (fns.destroy_tap)(tap_id);
                dlclose(lib);
            }
            let err = format!("System audio did not start ({status}).");
            report(&mut ready_tx, Err(err.clone()));
            return Err(err);
        }
        let sample_rate = nominal_rate(&fns, device_id);
        report(&mut ready_tx, Ok((sample_rate, 1)));
        while !stop.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(20));
        }
        unsafe {
            (fns.stop)(device_id, proc_id);
            (fns.destroy_io)(device_id, proc_id);
            drop(Box::from_raw(shared));
            (fns.destroy_aggregate)(device_id);
            (fns.destroy_tap)(tap_id);
            dlclose(lib);
        }
        Ok(())
    }

    fn nominal_rate(fns: &CoreFns, device_id: u32) -> u32 {
        let mut address = AudioObjectPropertyAddress {
            mSelector: kAudioDevicePropertyNominalSampleRate,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain,
        };
        let mut size = std::mem::size_of::<f64>() as u32;
        let mut rate = 48_000.0f64;
        let status = unsafe {
            (fns.get_property)(
                device_id,
                &address,
                0,
                std::ptr::null(),
                &mut size,
                &mut rate as *mut f64 as *mut c_void,
            )
        };
        let _ = &mut address;
        if status == 0 && rate.is_finite() && rate >= 8_000.0 {
            rate.round() as u32
        } else {
            48_000
        }
    }

    fn key_string(key: &CStr) -> CFString {
        CFString::new(key.to_str().unwrap_or(""))
    }

    fn cf_dict(pairs: &[(CFString, core_foundation::base::CFType)]) -> CFDictionary<CFString, core_foundation::base::CFType> {
        CFDictionary::from_CFType_pairs(pairs)
    }

    fn load_fns(lib: *mut c_void) -> Result<CoreFns, String> {
        Ok(CoreFns {
            create_tap: load_sym(lib, "AudioHardwareCreateProcessTap")?,
            destroy_tap: load_sym(lib, "AudioHardwareDestroyProcessTap")?,
            create_aggregate: load_sym(lib, "AudioHardwareCreateAggregateDevice")?,
            destroy_aggregate: load_sym(lib, "AudioHardwareDestroyAggregateDevice")?,
            create_io: load_sym(lib, "AudioDeviceCreateIOProcID")?,
            destroy_io: load_sym(lib, "AudioDeviceDestroyIOProcID")?,
            start: load_sym(lib, "AudioDeviceStart")?,
            stop: load_sym(lib, "AudioDeviceStop")?,
            get_property: load_sym(lib, "AudioObjectGetPropertyData")?,
        })
    }

    fn load_sym<T>(lib: *mut c_void, name: &str) -> Result<T, String> {
        assert_eq!(std::mem::size_of::<T>(), std::mem::size_of::<*mut c_void>());
        let c_name = CString::new(name).map_err(|err| err.to_string())?;
        let ptr = unsafe { dlsym(lib, c_name.as_ptr()) };
        if ptr.is_null() {
            return Err("System audio capture needs macOS 14.2 or later.".into());
        }
        Ok(unsafe { std::mem::transmute_copy(&ptr) })
    }

    unsafe extern "C" fn tap_io(
        _device: u32,
        _now: *const c_void,
        input: *const AudioBufferList,
        _in_time: *const c_void,
        _output: *mut c_void,
        _out_time: *const c_void,
        client: *mut c_void,
    ) -> i32 {
        if input.is_null() || client.is_null() {
            return 0;
        }
        let shared = &*(client as *const TapShared);
        let list = &*input;
        let count = list.mNumberBuffers as usize;
        if count == 0 {
            return 0;
        }
        let buffers = std::slice::from_raw_parts(list.mBuffers.as_ptr(), count);
        let mut pcm = Vec::<f32>::new();
        if count == 1 {
            append_interleaved(&mut pcm, &buffers[0]);
        } else {
            let frames = buffers.iter().map(|buf| {
                if buf.mData.is_null() || buf.mNumberChannels == 0 {
                    0
                } else {
                    buf.mDataByteSize as usize / 4 / buf.mNumberChannels.max(1) as usize
                }
            });
            let frames = frames.min().unwrap_or(0);
            for frame in 0..frames {
                let mut sum = 0.0f32;
                let mut channels = 0usize;
                for buf in buffers {
                    if buf.mData.is_null() {
                        continue;
                    }
                    let data = std::slice::from_raw_parts(buf.mData as *const f32, buf.mDataByteSize as usize / 4);
                    let ch = buf.mNumberChannels.max(1) as usize;
                    for channel in 0..ch {
                        let index = frame * ch + channel;
                        if let Some(sample) = data.get(index) {
                            sum += *sample;
                            channels += 1;
                        }
                    }
                }
                if channels > 0 {
                    pcm.push(sum / channels as f32);
                }
            }
        }
        if !pcm.is_empty() {
            if let Ok(mut lock) = shared.samples.lock() {
                lock.extend_from_slice(&pcm);
            }
        }
        0
    }

    fn append_interleaved(out: &mut Vec<f32>, buf: &AudioBuffer) {
        if buf.mData.is_null() || buf.mDataByteSize < 4 {
            return;
        }
        let data = unsafe { std::slice::from_raw_parts(buf.mData as *const f32, buf.mDataByteSize as usize / 4) };
        let channels = buf.mNumberChannels.max(1) as usize;
        if channels == 1 {
            out.extend_from_slice(data);
            return;
        }
        for frame in data.chunks(channels) {
            let sum: f32 = frame.iter().copied().sum();
            out.push(sum / frame.len() as f32);
        }
    }
}
