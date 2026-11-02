//! Bluetooth on macOS, through IOBluetooth: the phones this Mac knows, looking for and pairing
//! new ones, and RFCOMM channels to the phone's Message Access and Phonebook Access services
//! (spoken over with `goep`).
//!
//! IOBluetooth only opens channels from the main thread, and calls its delegates back there. So
//! every call here goes to the main thread (`run_on_main`, which the window's run loop serves),
//! the delegates hand what they're told to `Shared`, and the phone thread waits on that.

use std::ffi::c_void;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use dispatch2::{MainThreadBound, run_on_main};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{AnyThread, DefinedClass, define_class, msg_send, sel};
use objc2_foundation::{NSObjectProtocol, NSString};
use objc2_io_bluetooth::{IOBluetoothDevice, IOBluetoothDeviceInquiry, IOBluetoothDevicePair, IOBluetoothRFCOMMChannel, IOBluetoothSDPServiceRecord};

use crate::bluez::Device;
use crate::goep;

pub type Result<T> = std::result::Result<T, String>;

/// What the delegates have been told.
#[derive(Default)]
struct State {
    data: Vec<u8>,
    closed: bool,
    opened: Option<i32>,
    sdp: Option<i32>,
    code: Option<u32>,
    paired: Option<i32>,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

impl Shared {
    fn update(&self, f: impl FnOnce(&mut State)) {
        f(&mut self.state.lock().unwrap_or_else(|e| e.into_inner()));
        self.changed.notify_all();
    }

    /// Waits up to `timeout` for `f` to find what it wants.
    fn wait<T>(&self, timeout: Duration, mut f: impl FnMut(&mut State) -> Option<T>) -> Option<T> {
        let end = Instant::now() + timeout;
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(t) = f(&mut s) {
                return Some(t);
            }
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            s = self.changed.wait_timeout(s, left).unwrap_or_else(|e| e.into_inner()).0;
        }
    }
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and `Delegate` doesn't implement Drop.
    #[unsafe(super(NSObject))]
    #[name = "RunePhoneBluetoothDelegate"]
    #[ivars = Arc<Shared>]
    struct Delegate;

    // IOBluetooth asks whether its delegate responds to each of these, so no protocol is declared.
    impl Delegate {
        #[unsafe(method(rfcommChannelData:data:length:))]
        fn rfcomm_data(&self, _channel: *mut AnyObject, data: *mut c_void, length: usize) {
            if data.is_null() {
                return;
            }
            // SAFETY: IOBluetooth hands over `length` bytes at `data`, valid for this call.
            let bytes = unsafe { std::slice::from_raw_parts(data as *const u8, length) };
            self.ivars().update(|s| s.data.extend_from_slice(bytes));
        }

        #[unsafe(method(rfcommChannelOpenComplete:status:))]
        fn rfcomm_opened(&self, _channel: *mut AnyObject, status: i32) {
            self.ivars().update(|s| s.opened = Some(status));
        }

        #[unsafe(method(rfcommChannelClosed:))]
        fn rfcomm_closed(&self, _channel: *mut AnyObject) {
            self.ivars().update(|s| s.closed = true);
        }

        #[unsafe(method(sdpQueryComplete:status:))]
        fn sdp_done(&self, _device: *mut AnyObject, status: i32) {
            self.ivars().update(|s| s.sdp = Some(status));
        }

        #[unsafe(method(devicePairingUserConfirmationRequest:numericValue:))]
        fn pairing_code(&self, _sender: *mut AnyObject, value: u32) {
            self.ivars().update(|s| s.code = Some(value));
        }

        #[unsafe(method(devicePairingFinished:error:))]
        fn pairing_done(&self, _sender: *mut AnyObject, error: i32) {
            self.ivars().update(|s| s.paired = Some(error));
        }
    }

    unsafe impl NSObjectProtocol for Delegate {}
);

impl Delegate {
    fn new(shared: Arc<Shared>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(shared);
        // SAFETY: NSObject's init, on an object whose ivars are set.
        unsafe { msg_send![super(this), init] }
    }
}

/// IOBluetooth writes addresses as `34-ba-9a-d0-30-38`; the app, like BlueZ, as `34:BA:9A:D0:30:38`.
fn address_of(d: &IOBluetoothDevice) -> String {
    // SAFETY: a plain getter.
    unsafe { d.addressString() }.map(|s| s.to_string().replace('-', ":").to_uppercase()).unwrap_or_default()
}

fn describe(d: &IOBluetoothDevice) -> Device {
    let address = address_of(d);
    // SAFETY: plain getters.
    let (name, paired, connected, class) = unsafe {
        (d.name().to_string(), d.isPaired(), d.isConnected(), d.classOfDevice())
    };
    Device {
        path: address.clone(),
        name: if name.is_empty() { address.clone() } else { name },
        address,
        paired,
        connected,
        // Major device class 2: a phone.
        phone: (class >> 8) & 0x1F == 2,
    }
}

/// The device with this address (on the main thread).
fn device(address: &str) -> Option<Retained<IOBluetoothDevice>> {
    // SAFETY: a class method taking a string.
    unsafe { IOBluetoothDevice::deviceWithAddressString(Some(&NSString::from_str(address))) }
}

/// The devices this Mac is paired with, each once.
pub fn paired() -> Vec<Device> {
    run_on_main(|_| {
        let mut out: Vec<Device> = Vec::new();
        // SAFETY: a class method; the array holds IOBluetoothDevices.
        let Some(all) = (unsafe { IOBluetoothDevice::pairedDevices() }) else { return out };
        for d in all.iter() {
            if let Ok(d) = d.downcast::<IOBluetoothDevice>() {
                let d = describe(&d);
                if !out.iter().any(|k| k.address == d.address) {
                    out.push(d);
                }
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    })
}

/// Unpairs a device. macOS has no public call for it; the one System Settings uses is tried,
/// and failing that the person is sent there.
pub fn remove(address: &str) -> Result<()> {
    let done = run_on_main(|_| {
        let Some(d) = device(address) else { return false };
        if !d.respondsToSelector(sel!(remove)) {
            return false;
        }
        // SAFETY: `remove` takes nothing and returns nothing.
        let _: () = unsafe { msg_send![&*d, remove] };
        true
    });
    if done { Ok(()) } else { Err("Remove the phone in System Settings › Bluetooth".into()) }
}

/// A search for devices nearby, stopped when dropped.
pub struct Inquiry {
    inquiry: MainThreadBound<(Retained<IOBluetoothDeviceInquiry>, Retained<Delegate>)>,
}

impl Inquiry {
    pub fn start() -> Result<Inquiry> {
        let inquiry = run_on_main(|mtm| {
            let delegate = Delegate::new(Arc::default());
            // SAFETY: the delegate outlives the inquiry (they're kept together).
            let inquiry = unsafe { IOBluetoothDeviceInquiry::inquiryWithDelegate(Some(&delegate)) }.ok_or("Bluetooth isn't available")?;
            // SAFETY: plain setters and `start`.
            let r = unsafe {
                inquiry.setUpdateNewDeviceNames(true);
                inquiry.start()
            };
            if r != 0 {
                return Err(format!("Couldn't look for phones (IOBluetooth 0x{r:08X})"));
            }
            Ok(MainThreadBound::new((inquiry, delegate), mtm))
        })?;
        Ok(Inquiry { inquiry })
    }

    /// What it has found so far.
    pub fn found(&self) -> Vec<Device> {
        self.inquiry.get_on_main(|(inquiry, _)| {
            // SAFETY: a plain getter; the array holds IOBluetoothDevices.
            let Some(all) = (unsafe { inquiry.foundDevices() }) else { return Vec::new() };
            all.iter().filter_map(|d| d.downcast::<IOBluetoothDevice>().ok()).map(|d| describe(&d)).collect()
        })
    }
}

impl Drop for Inquiry {
    fn drop(&mut self) {
        // SAFETY: stopping an inquiry that may already have finished is fine.
        self.inquiry.get_on_main(|(inquiry, _)| unsafe {
            inquiry.stop();
        });
    }
}

/// Pairing with a phone, numeric comparison: the code is shown on both screens and the person
/// says whether they match.
pub struct Pairing {
    pair: MainThreadBound<(Retained<IOBluetoothDevicePair>, Retained<Delegate>)>,
    shared: Arc<Shared>,
}

impl Pairing {
    pub fn start(address: &str) -> Result<Pairing> {
        let shared = Arc::new(Shared::default());
        let s = shared.clone();
        let address = address.to_owned();
        let pair = run_on_main(move |mtm| {
            let d = device(&address).ok_or("That phone is gone")?;
            let delegate = Delegate::new(s);
            // SAFETY: the delegate outlives the pairing (they're kept together).
            let pair = unsafe { IOBluetoothDevicePair::pairWithDevice(Some(&d)) }.ok_or("Couldn't start pairing")?;
            let r = unsafe {
                pair.setDelegate(Some(&delegate));
                pair.start()
            };
            if r != 0 {
                return Err(format!("Couldn't start pairing (IOBluetooth 0x{r:08X})"));
            }
            Ok(MainThreadBound::new((pair, delegate), mtm))
        })?;
        Ok(Pairing { pair, shared })
    }

    /// The code to compare, once the phone has sent it.
    pub fn code(&self) -> Option<u32> {
        self.shared.wait(Duration::ZERO, |s| s.code.take())
    }

    /// How it ended (an IOReturn, 0 for paired), once it has.
    pub fn finished(&self) -> Option<i32> {
        self.shared.wait(Duration::ZERO, |s| s.paired)
    }

    pub fn answer(&self, matches: bool) {
        // SAFETY: answering the confirmation request the delegate was given.
        self.pair.get_on_main(|(pair, _)| unsafe { pair.replyUserConfirmation(matches) });
    }
}

impl Drop for Pairing {
    fn drop(&mut self) {
        if self.finished().is_none() {
            // SAFETY: stopping a pairing still under way.
            self.pair.get_on_main(|(pair, _)| unsafe { pair.stop() });
        }
    }
}

/// An RFCOMM channel to one of the phone's services; closed when dropped.
pub struct Rfcomm {
    channel: MainThreadBound<(Retained<IOBluetoothRFCOMMChannel>, Retained<Delegate>)>,
    shared: Arc<Shared>,
    mtu: usize,
}

/// Opens a channel to the service with class UUID `service` on the phone at `address`. The phone
/// is asked for its services first: they say the channel, and macOS won't open one without it.
pub fn rfcomm(address: &str, service: u16) -> Result<Rfcomm> {
    // macOS can leave the first channel to a phone waiting forever, without asking the phone at
    // all; a second try goes through.
    match open(address, service) {
        Err(e) if e == NO_ANSWER => open(address, service),
        r => r,
    }
}

const NO_ANSWER: &str = "The phone didn't answer";

fn open(address: &str, service: u16) -> Result<Rfcomm> {
    let shared = Arc::new(Shared::default());
    let s = shared.clone();
    let a = address.to_owned();
    let delegate = run_on_main(move |mtm| {
        let d = device(&a).ok_or("This Mac doesn't know that phone")?;
        let delegate = Delegate::new(s);
        // SAFETY: the delegate is kept until the query is answered (it's returned).
        let r = unsafe { d.performSDPQuery(Some(&delegate)) };
        if r != 0 {
            return Err("The phone isn't in reach".to_string());
        }
        Ok(MainThreadBound::new(delegate, mtm))
    })?;
    let status = shared.wait(Duration::from_secs(20), |s| s.sdp).ok_or("The phone isn't in reach")?;
    if status != 0 {
        return Err("The phone isn't in reach".into());
    }
    let a = address.to_owned();
    let id = run_on_main(move |_| {
        let d = device(&a)?;
        // SAFETY: plain getters; the array holds service records.
        let services = unsafe { d.services() }?;
        services.iter().filter_map(|r| r.downcast::<IOBluetoothSDPServiceRecord>().ok()).find_map(|r| unsafe {
            let mut id = 0u8;
            (r.matchesUUID16(service) && r.getRFCOMMChannelID(&mut id) == 0).then_some(id)
        })
    })
    .ok_or("The phone doesn't offer that over Bluetooth")?;
    let a = address.to_owned();
    let channel = run_on_main(move |mtm| {
        let d = device(&a).ok_or("This Mac doesn't know that phone")?;
        let delegate = delegate.into_inner(mtm);
        let mut channel = None;
        // SAFETY: the delegate is kept as long as the channel (they're kept together).
        let r = unsafe { d.openRFCOMMChannelAsync_withChannelID_delegate(Some(&mut channel), id, Some(&delegate)) };
        match channel {
            Some(c) if r == 0 => Ok(MainThreadBound::new((c, delegate), mtm)),
            _ => Err(format!("Couldn't reach the phone (IOBluetooth 0x{r:08X})")),
        }
    })?;
    // The phone accepts at once (asking the person comes after, when the session connects).
    let Some(opened) = shared.wait(Duration::from_secs(15), |s| s.opened) else {
        // SAFETY: closing a channel still opening is fine.
        channel.get_on_main(|(c, _)| unsafe {
            c.closeChannel();
        });
        return Err(NO_ANSWER.into());
    };
    if opened != 0 {
        return Err(format!("Couldn't reach the phone (IOBluetooth 0x{opened:08X})"));
    }
    // SAFETY: a plain getter.
    let mtu = channel.get_on_main(|(c, _)| unsafe { c.getMTU() }) as usize;
    Ok(Rfcomm { channel, shared, mtu: mtu.max(16) })
}

impl goep::Stream for Rfcomm {
    fn write(&mut self, data: &[u8]) -> Result<()> {
        for chunk in data.chunks(self.mtu) {
            let mut chunk = chunk.to_vec();
            // SAFETY: `chunk` is valid for its length for the whole (synchronous) write.
            let r = self.channel.get_on_main(|(c, _)| unsafe { c.writeSync_length(chunk.as_mut_ptr() as *mut c_void, chunk.len() as u16) });
            if r != 0 {
                return Err(goep::GONE.into());
            }
        }
        Ok(())
    }

    fn read(&mut self, timeout: Duration) -> Result<Vec<u8>> {
        let got = self.shared.wait(timeout, |s| {
            if !s.data.is_empty() {
                Some(Ok(std::mem::take(&mut s.data)))
            } else if s.closed {
                Some(Err(goep::GONE.to_string()))
            } else {
                None
            }
        });
        got.unwrap_or(Ok(Vec::new()))
    }
}

impl Drop for Rfcomm {
    fn drop(&mut self) {
        // SAFETY: closing a channel that may already be closed is fine.
        self.channel.get_on_main(|(c, _)| unsafe {
            c.closeChannel();
        });
    }
}
