//! One esp-mqtt client, through ESP-IDF's C API, and the handler that turns
//! what it dispatches into events. The C API is used directly because the
//! CONNACK return code of a refusal is only in the raw error event, and a
//! handler registered after `esp_mqtt_client_start` could miss it.

use std::ffi::{c_char, c_int, c_void, CStr, CString, NulError};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::{mem, ptr, slice};

use connectors::{BrokerAddress, BrokerError, ConnectOptions, Event, Incoming, Publication};
use esp_idf_svc::sys::{self, esp, EspError};

use crate::events::EventSender;
use crate::threads;

/// Every publication, subscription, and will is QoS 1.
const QOS: c_int = 1;
/// The receive and send buffers. A message longer than the receive buffer
/// arrives in pieces, which `Reception` puts back together.
const BUFFER_BYTES: c_int = 4096;
/// The client's task runs the TLS handshake and this module's handler.
const TASK_STACK_BYTES: c_int = 8 * 1024;

/// What a client is opened with, owned so that the C strings outlive it:
/// esp-mqtt copies everything but the CA certificate, which it reads on
/// every connection attempt.
pub(super) struct Settings {
    address: Address,
    login: Login,
    will: Will,
    keep_alive_s: u16,
}

struct Address {
    host: CString,
    port: u16,
    tls: bool,
    ca_pem: CString,
}

struct Login {
    client_id: CString,
    username: CString,
    password: CString,
}

struct Will {
    topic: CString,
    /// The payload, followed by a NUL so that an empty one is still a
    /// valid C string.
    message: Vec<u8>,
    length: c_int,
    retain: bool,
}

impl Settings {
    pub(super) fn new(options: &ConnectOptions) -> Result<Self, NulError> {
        Ok(Self {
            address: Address::new(&options.address)?,
            login: Login::new(options)?,
            will: Will::new(&options.will)?,
            keep_alive_s: options.keep_alive_s,
        })
    }

    /// MQTT 3.1.1, clean session, keep-alive and will as given, and no
    /// reconnection of its own: the application decides when to connect
    /// again.
    fn config(&self) -> sys::esp_mqtt_client_config_t {
        sys::esp_mqtt_client_config_t {
            broker: self.address.broker(),
            credentials: self.login.credentials(),
            session: sys::esp_mqtt_client_config_t_session_t {
                last_will: self.will.last_will(),
                disable_clean_session: false,
                keepalive: self.keep_alive_s.into(),
                disable_keepalive: false,
                protocol_ver: sys::esp_mqtt_protocol_ver_t_MQTT_PROTOCOL_V_3_1_1,
                ..Default::default()
            },
            network: sys::esp_mqtt_client_config_t_network_t {
                disable_auto_reconnect: true,
                ..Default::default()
            },
            task: sys::esp_mqtt_client_config_t_task_t {
                stack_size: TASK_STACK_BYTES,
                ..Default::default()
            },
            buffer: sys::esp_mqtt_client_config_t_buffer_t {
                size: BUFFER_BYTES,
                ..Default::default()
            },
            ..Default::default()
        }
    }
}

impl Address {
    fn new(address: &BrokerAddress) -> Result<Self, NulError> {
        Ok(Self {
            host: CString::new(address.host.as_str())?,
            port: address.port,
            tls: address.tls,
            ca_pem: CString::new(address.ca_pem.as_str())?,
        })
    }

    /// The broker's address, over TLS verified against the CA when `tls`
    /// is set, and plain TCP otherwise.
    fn broker(&self) -> sys::esp_mqtt_client_config_t_broker_t {
        let (transport, certificate) = match self.tls {
            true => (
                sys::esp_mqtt_transport_t_MQTT_TRANSPORT_OVER_SSL,
                self.ca_pem.as_ptr(),
            ),
            false => (
                sys::esp_mqtt_transport_t_MQTT_TRANSPORT_OVER_TCP,
                ptr::null(),
            ),
        };
        sys::esp_mqtt_client_config_t_broker_t {
            address: sys::esp_mqtt_client_config_t_broker_t_address_t {
                hostname: self.host.as_ptr(),
                transport,
                port: self.port.into(),
                ..Default::default()
            },
            // A length of 0 marks the certificate as NUL-terminated PEM.
            verification: sys::esp_mqtt_client_config_t_broker_t_verification_t {
                certificate,
                certificate_len: 0,
                ..Default::default()
            },
        }
    }
}

impl Login {
    fn new(options: &ConnectOptions) -> Result<Self, NulError> {
        Ok(Self {
            client_id: CString::new(options.client_id.as_str())?,
            username: CString::new(options.username.as_str())?,
            password: CString::new(options.password.as_str())?,
        })
    }

    fn credentials(&self) -> sys::esp_mqtt_client_config_t_credentials_t {
        sys::esp_mqtt_client_config_t_credentials_t {
            username: self.username.as_ptr(),
            client_id: self.client_id.as_ptr(),
            authentication: sys::esp_mqtt_client_config_t_credentials_t_authentication_t {
                password: self.password.as_ptr(),
                ..Default::default()
            },
            ..Default::default()
        }
    }
}

impl Will {
    fn new(will: &Publication) -> Result<Self, NulError> {
        let mut message = will.payload.clone();
        message.push(0);
        Ok(Self {
            topic: CString::new(will.topic.as_str())?,
            length: c_int::try_from(will.payload.len()).unwrap_or(c_int::MAX),
            message,
            retain: will.retain,
        })
    }

    fn last_will(&self) -> sys::esp_mqtt_client_config_t_session_t_last_will_t {
        sys::esp_mqtt_client_config_t_session_t_last_will_t {
            topic: self.topic.as_ptr(),
            msg: self.message.as_ptr().cast(),
            msg_len: self.length,
            qos: QOS,
            retain: self.retain.into(),
        }
    }
}

/// A running client. Dropping it closes the connection with DISCONNECT
/// when it is connected, so the broker keeps the will, and waits for the
/// client's task to end; it must not be dropped on that task.
pub(super) struct Client {
    handle: sys::esp_mqtt_client_handle_t,
    generation: u32,
    /// Holds the CA certificate the handle points to.
    _settings: Settings,
    /// The handler's argument; boxed so that its address stays put.
    reporter: Box<Reporter>,
}

// SAFETY: esp-mqtt guards every call on a handle with the client's own
// lock, so the handle may be used and destroyed from any thread but the
// client's task, which this connector never calls it from.
unsafe impl Send for Client {}

impl Client {
    /// Creates a client for `settings` and starts its task, which connects
    /// and reports through `reporter`.
    pub(super) fn start(settings: Settings, reporter: Reporter) -> Result<Self, EspError> {
        let config = settings.config();
        // SAFETY: the configuration points into `settings`, which the
        // client keeps; esp-mqtt copies what it does not keep a pointer to.
        let handle = unsafe { sys::esp_mqtt_client_init(&config) };
        if handle.is_null() {
            return Err(EspError::from_infallible::<{ sys::ESP_FAIL }>());
        }
        let client = Self {
            handle,
            generation: reporter.generation,
            _settings: settings,
            reporter: Box::new(reporter),
        };
        client.register()?;
        // SAFETY: the handle was just created and has not been started.
        esp!(unsafe { sys::esp_mqtt_client_start(client.handle) })?;
        Ok(client)
    }

    fn register(&self) -> Result<(), EspError> {
        let argument = ptr::from_ref::<Reporter>(&self.reporter).cast_mut();
        // SAFETY: the reporter lives in a box this client owns, and the
        // handle, with the registration, is destroyed before the box.
        esp!(unsafe {
            sys::esp_mqtt_client_register_event(
                self.handle,
                sys::esp_mqtt_event_id_t_MQTT_EVENT_ANY,
                Some(on_event),
                argument.cast(),
            )
        })
    }

    /// The number of the connection this client was opened for.
    pub(super) fn generation(&self) -> u32 {
        self.generation
    }

    pub(super) fn subscribe(&self, topic: &CStr) -> Result<(), BrokerError> {
        // SAFETY: the topic is NUL-terminated and esp-mqtt copies it into
        // the SUBSCRIBE packet before returning.
        let id = unsafe { sys::esp_mqtt_client_subscribe_single(self.handle, topic.as_ptr(), QOS) };
        message_id(id).map(drop)
    }

    /// Queues the publication for the client's task to send, so the caller
    /// never waits on the network, and returns the message id esp-mqtt
    /// gave it, which the PUBACK event quotes.
    pub(super) fn publish(
        &self,
        topic: &CStr,
        payload: &[u8],
        retain: bool,
    ) -> Result<u16, BrokerError> {
        let length = c_int::try_from(payload.len())
            .map_err(|_| BrokerError("the payload is too long".to_owned()))?;
        // SAFETY: the topic is NUL-terminated, the payload is `length`
        // bytes or null when empty, and esp-mqtt copies both into its
        // outbox before returning.
        let id = unsafe {
            sys::esp_mqtt_client_enqueue(
                self.handle,
                topic.as_ptr(),
                payload_pointer(payload),
                length,
                QOS,
                retain.into(),
                true,
            )
        };
        message_id(id)
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // SAFETY: the handle is this client's, destroyed once, and never
        // from the client's own task (see the type's documentation).
        unsafe { sys::esp_mqtt_client_destroy(self.handle) };
    }
}

/// esp-mqtt reads a null payload as empty, and a non-null one of length 0
/// as a C string.
fn payload_pointer(payload: &[u8]) -> *const c_char {
    match payload.is_empty() {
        true => ptr::null(),
        false => payload.as_ptr().cast(),
    }
}

/// The message id esp-mqtt returned, or what it said instead: -1 for a
/// failure, -2 for a full outbox.
fn message_id(id: c_int) -> Result<u16, BrokerError> {
    u16::try_from(id).map_err(|_| BrokerError(format!("esp-mqtt returned {id}")))
}

/// Reports what one client's task dispatches, while its connection is the
/// one last asked for.
pub(super) struct Reporter {
    events: EventSender,
    generation: u32,
    current: Arc<AtomicU32>,
    reception: Mutex<Reception>,
}

/// What the handler carries from one event to the next; only the client's
/// task touches it.
#[derive(Default)]
struct Reception {
    /// A message whose pieces are still arriving.
    partial: Option<Incoming>,
    /// The broker refused the connection, so the disconnection esp-mqtt
    /// dispatches next is already reported.
    refused: bool,
}

impl Reporter {
    /// Reports for connection number `generation`, while `current` holds
    /// that number.
    pub(super) fn new(events: EventSender, generation: u32, current: Arc<AtomicU32>) -> Self {
        Self {
            events,
            generation,
            current,
            reception: Mutex::default(),
        }
    }

    pub(super) fn report(&self, event: Event) {
        if self.current.load(Ordering::Acquire) == self.generation {
            self.events.push(event);
        }
    }

    fn reception(&self) -> MutexGuard<'_, Reception> {
        threads::lock(&self.reception)
    }

    /// Events about the connection itself, then the messages it carries.
    fn observe(&self, event: &sys::esp_mqtt_event_t) {
        match event.event_id {
            sys::esp_mqtt_event_id_t_MQTT_EVENT_CONNECTED => self.report(Event::Connected),
            sys::esp_mqtt_event_id_t_MQTT_EVENT_DISCONNECTED => self.disconnected(),
            sys::esp_mqtt_event_id_t_MQTT_EVENT_ERROR => self.failed(event),
            _ => self.carried(event),
        }
    }

    /// A message received, or the broker's acknowledgement of one sent.
    fn carried(&self, event: &sys::esp_mqtt_event_t) {
        match event.event_id {
            sys::esp_mqtt_event_id_t_MQTT_EVENT_DATA => self.received(event),
            sys::esp_mqtt_event_id_t_MQTT_EVENT_PUBLISHED => self.delivered(event.msg_id),
            _ => {}
        }
    }

    fn disconnected(&self) {
        let refused = mem::take(&mut self.reception().refused);
        if !refused {
            self.report(Event::Disconnected);
        }
    }

    fn delivered(&self, id: c_int) {
        if let Ok(message_id) = u16::try_from(id) {
            self.report(Event::Delivered { message_id });
        }
    }

    fn received(&self, event: &sys::esp_mqtt_event_t) {
        let whole = self.reception().assemble(event);
        if let Some(message) = whole {
            self.report(Event::Message(message));
        }
    }

    /// Reports a refusal with its CONNACK return code, and logs any other
    /// failure, whose disconnection esp-mqtt dispatches next.
    fn failed(&self, event: &sys::esp_mqtt_event_t) {
        // SAFETY: the error handle is null or points to the client's error
        // codes, which stay put while the event is dispatched.
        let Some(codes) = (unsafe { event.error_handle.as_ref() }) else {
            return;
        };
        if codes.error_type != sys::esp_mqtt_error_type_t_MQTT_ERROR_TYPE_CONNECTION_REFUSED {
            log_failure(codes);
            return;
        }
        self.reception().refused = true;
        let code = u8::try_from(codes.connect_return_code).unwrap_or(u8::MAX);
        self.report(Event::Refused { code });
    }
}

fn log_failure(codes: &sys::esp_mqtt_error_codes_t) {
    log::warn!(
        "broker connection: esp-tls error {:#x}, TLS stack error {:#x}, certificate flags {:#x}, socket errno {}",
        codes.esp_tls_last_esp_err,
        codes.esp_tls_stack_err,
        codes.esp_tls_cert_verify_flags,
        codes.esp_transport_sock_errno,
    );
}

impl Reception {
    /// Adds the piece of a message `event` carries, and returns the message
    /// once it is whole. Only the first piece carries the topic.
    fn assemble(&mut self, event: &sys::esp_mqtt_event_t) -> Option<Incoming> {
        if event.current_data_offset == 0 {
            self.partial = Some(Incoming {
                topic: topic(event),
                payload: Vec::new(),
            });
        }
        let message = self.partial.as_mut()?;
        // SAFETY: the data is `data_len` bytes in the client's buffer,
        // unchanged while the event is dispatched.
        message
            .payload
            .extend_from_slice(unsafe { bytes(event.data, event.data_len) });
        let total = usize::try_from(event.total_data_len).unwrap_or(0);
        if message.payload.len() < total {
            return None;
        }
        self.partial.take()
    }
}

fn topic(event: &sys::esp_mqtt_event_t) -> String {
    // SAFETY: the topic is `topic_len` bytes in the client's buffer,
    // unchanged while the event is dispatched.
    let topic = unsafe { bytes(event.topic, event.topic_len) };
    String::from_utf8_lossy(topic).into_owned()
}

/// The `length` bytes at `start`; none when `start` is null.
///
/// # Safety
///
/// A non-null `start` points to `length` readable bytes that outlive the
/// returned slice.
unsafe fn bytes<'a>(start: *const c_char, length: c_int) -> &'a [u8] {
    let length = usize::try_from(length).unwrap_or(0);
    if start.is_null() || length == 0 {
        return &[];
    }
    // SAFETY: guaranteed by the caller.
    unsafe { slice::from_raw_parts(start.cast(), length) }
}

/// The handler esp-mqtt calls on the client's task for every event.
///
/// # Safety
///
/// `argument` is the `Reporter` the client registered, and `data` the
/// `esp_mqtt_event_t` being dispatched.
unsafe extern "C" fn on_event(
    argument: *mut c_void,
    _base: sys::esp_event_base_t,
    _id: i32,
    data: *mut c_void,
) {
    // SAFETY: guaranteed by the registration in `Client::register`: the
    // reporter outlives the handle, and esp-mqtt passes the event it
    // dispatches, valid for the call.
    let (reporter, event) = unsafe {
        (
            &*argument.cast_const().cast::<Reporter>(),
            data.cast_const().cast::<sys::esp_mqtt_event_t>().as_ref(),
        )
    };
    if let Some(event) = event {
        reporter.observe(event);
    }
}
