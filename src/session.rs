//! Minimal X11R6 Session Management (XSMP) client.
//!
//! KDE's `ksmserver` asks running applications to quit at logout over XSMP (an
//! ICE connection named by `$SESSION_MANAGER`) — this is what every session
//! aware toolkit (Qt, GTK) does. Registering here lets the app be told to quit
//! *regardless of its window state*: minimized, hidden to the tray, or visible.
//! A Wayland toplevel close can't do that, because KWin withholds `close` from a
//! window that isn't activated, so a backgrounded window otherwise blocks logout.
//!
//! We handle the two messages that matter: `SaveYourself` (reply "done" — the
//! app has no session state to save; the config is persisted eagerly on edit)
//! and `Die` (exit the process). The ICE connection is pumped on a dedicated
//! thread.

use std::ffi::{CString, c_char, c_int, c_ulong, c_void};
use std::ptr;

use log::info;

type SmcConn = *mut c_void;
type IceConn = *mut c_void;
type SmPointer = *mut c_void;

#[repr(C)]
struct SmcCallback {
    callback: *mut c_void,
    client_data: SmPointer,
}

#[repr(C)]
struct SmcCallbacks {
    save_yourself: SmcCallback,
    die: SmcCallback,
    save_complete: SmcCallback,
    shutdown_cancelled: SmcCallback,
}

#[repr(C)]
struct SmPropValue {
    length: c_int,
    value: SmPointer,
}

#[repr(C)]
struct SmProp {
    name: *mut c_char,
    type_: *mut c_char,
    num_vals: c_int,
    vals: *mut SmPropValue,
}

const SM_SAVE_YOURSELF_MASK: c_ulong = 1 << 0;
const SM_DIE_MASK: c_ulong = 1 << 1;
const SM_SAVE_COMPLETE_MASK: c_ulong = 1 << 2;
const SM_SHUTDOWN_CANCELLED_MASK: c_ulong = 1 << 3;

const ICE_PROCESS_MESSAGES_SUCCESS: c_int = 0;

#[link(name = "SM")]
unsafe extern "C" {
    fn SmcOpenConnection(
        network_ids: *const c_char,
        context: SmPointer,
        major: c_int,
        minor: c_int,
        mask: c_ulong,
        callbacks: *mut SmcCallbacks,
        previous_id: *const c_char,
        client_id_ret: *mut *mut c_char,
        error_len: c_int,
        error_ret: *mut c_char,
    ) -> SmcConn;
    fn SmcSaveYourselfDone(conn: SmcConn, success: c_int);
    fn SmcSetProperties(conn: SmcConn, num_props: c_int, props: *mut *mut SmProp);
    fn SmcGetIceConnection(conn: SmcConn) -> IceConn;
}

#[link(name = "ICE")]
unsafe extern "C" {
    fn IceConnectionNumber(conn: IceConn) -> c_int;
    fn IceProcessMessages(conn: IceConn, reply_wait: *mut c_void, reply_ready: *mut c_int) -> c_int;
}

unsafe extern "C" fn on_save_yourself(
    conn: SmcConn,
    _client_data: SmPointer,
    _save_type: c_int,
    _shutdown: c_int,
    _interact_style: c_int,
    _fast: c_int,
) {
    // Nothing to save; acknowledge immediately so the session manager proceeds.
    unsafe { SmcSaveYourselfDone(conn, 1) };
}

unsafe extern "C" fn on_die(_conn: SmcConn, _client_data: SmPointer) {
    // The session is ending. Exit the process directly — this works regardless of
    // window state and tears down the ICE connection, which the session manager
    // reads as "client gone".
    std::process::exit(0);
}

unsafe extern "C" fn on_noop(_conn: SmcConn, _client_data: SmPointer) {}

/// Raw connection pointers, moved onto the ICE pump thread. Only ever touched
/// from that one thread, so the `Send` promise holds.
struct IcePump {
    ice: IceConn,
    fd: c_int,
}
unsafe impl Send for IcePump {}

/// Register with the session manager if one is present. Returns an error (rather
/// than panicking) when there is no `$SESSION_MANAGER` or the connection fails,
/// so the app can keep running without session management.
pub fn connect() -> Result<(), String> {
    if std::env::var_os("SESSION_MANAGER").is_none() {
        return Err("no SESSION_MANAGER in environment".to_owned());
    }

    let mut callbacks = SmcCallbacks {
        save_yourself: SmcCallback {
            callback: on_save_yourself as *mut c_void,
            client_data: ptr::null_mut(),
        },
        die: SmcCallback {
            callback: on_die as *mut c_void,
            client_data: ptr::null_mut(),
        },
        save_complete: SmcCallback {
            callback: on_noop as *mut c_void,
            client_data: ptr::null_mut(),
        },
        shutdown_cancelled: SmcCallback {
            callback: on_noop as *mut c_void,
            client_data: ptr::null_mut(),
        },
    };

    let mask =
        SM_SAVE_YOURSELF_MASK | SM_DIE_MASK | SM_SAVE_COMPLETE_MASK | SM_SHUTDOWN_CANCELLED_MASK;
    let mut client_id_ret: *mut c_char = ptr::null_mut();
    let mut error_buf = [0 as c_char; 256];

    let conn = unsafe {
        SmcOpenConnection(
            ptr::null(),
            ptr::null_mut(),
            1, // SmProtoMajor
            0, // SmProtoMinor
            mask,
            &mut callbacks,
            ptr::null(),
            &mut client_id_ret,
            error_buf.len() as c_int,
            error_buf.as_mut_ptr(),
        )
    };

    if conn.is_null() {
        let msg = unsafe { std::ffi::CStr::from_ptr(error_buf.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        return Err(format!("SmcOpenConnection failed: {msg}"));
    }

    set_properties(conn);

    let ice = unsafe { SmcGetIceConnection(conn) };
    let fd = unsafe { IceConnectionNumber(ice) };
    let pump = IcePump { ice, fd };

    std::thread::Builder::new()
        .name("xsmp-session".to_owned())
        .spawn(move || run_ice_pump(pump))
        .map_err(|err| format!("failed to spawn session thread: {err}"))?;

    info!("registered with the session manager (XSMP)");
    Ok(())
}

/// Block on the ICE socket and process session-management messages until the
/// connection drops (e.g. after `Die`).
fn run_ice_pump(pump: IcePump) {
    loop {
        let mut pfd = libc::pollfd {
            fd: pump.fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut pfd, 1, -1) };
        if ready < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            break;
        }
        if pfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
            break;
        }
        if pfd.revents & libc::POLLIN != 0 {
            let status =
                unsafe { IceProcessMessages(pump.ice, ptr::null_mut(), ptr::null_mut()) };
            if status != ICE_PROCESS_MESSAGES_SUCCESS {
                break; // IOError or ConnectionClosed
            }
        }
    }
}

/// Set the properties `ksmserver` expects; without at least `RestartCommand`
/// some session managers drop the client shortly after connecting.
fn set_properties(conn: SmcConn) {
    let program = std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(str::to_owned))
        .unwrap_or_else(|| "pipemeeter".to_owned());
    let user = std::env::var("USER").unwrap_or_else(|_| "user".to_owned());

    // Keep every CString alive until after SmcSetProperties copies the data.
    let name_program = CString::new("Program").unwrap();
    let name_user = CString::new("UserID").unwrap();
    let name_restart = CString::new("RestartCommand").unwrap();
    let name_clone = CString::new("CloneCommand").unwrap();
    let name_style = CString::new("RestartStyleHint").unwrap();
    let type_array8 = CString::new("ARRAY8").unwrap();
    let type_list = CString::new("LISTofARRAY8").unwrap();
    let type_card8 = CString::new("CARD8").unwrap();

    let program_c = CString::new(program.clone()).unwrap();
    let user_c = CString::new(user).unwrap();
    // SmRestartNever: this is a user-launched utility; don't auto-restore it on
    // next login (and don't try to relaunch a possibly-wrapped binary path).
    let mut restart_never: u8 = 3;

    let mut program_val = SmPropValue {
        length: program_c.as_bytes().len() as c_int,
        value: program_c.as_ptr() as SmPointer,
    };
    let mut user_val = SmPropValue {
        length: user_c.as_bytes().len() as c_int,
        value: user_c.as_ptr() as SmPointer,
    };
    // RestartCommand / CloneCommand are the argv to relaunch with: just the exe.
    let mut cmd_val = SmPropValue {
        length: program_c.as_bytes().len() as c_int,
        value: program_c.as_ptr() as SmPointer,
    };

    let mut prop_program = SmProp {
        name: name_program.as_ptr() as *mut c_char,
        type_: type_array8.as_ptr() as *mut c_char,
        num_vals: 1,
        vals: &mut program_val,
    };
    let mut prop_user = SmProp {
        name: name_user.as_ptr() as *mut c_char,
        type_: type_array8.as_ptr() as *mut c_char,
        num_vals: 1,
        vals: &mut user_val,
    };
    let mut prop_restart = SmProp {
        name: name_restart.as_ptr() as *mut c_char,
        type_: type_list.as_ptr() as *mut c_char,
        num_vals: 1,
        vals: &mut cmd_val,
    };
    let mut prop_clone = SmProp {
        name: name_clone.as_ptr() as *mut c_char,
        type_: type_list.as_ptr() as *mut c_char,
        num_vals: 1,
        vals: &mut cmd_val,
    };
    let mut style_val = SmPropValue {
        length: 1,
        value: &mut restart_never as *mut u8 as SmPointer,
    };
    let mut prop_style = SmProp {
        name: name_style.as_ptr() as *mut c_char,
        type_: type_card8.as_ptr() as *mut c_char,
        num_vals: 1,
        vals: &mut style_val,
    };

    let mut props: [*mut SmProp; 5] = [
        &mut prop_program,
        &mut prop_user,
        &mut prop_restart,
        &mut prop_clone,
        &mut prop_style,
    ];
    unsafe { SmcSetProperties(conn, props.len() as c_int, props.as_mut_ptr()) };
}
