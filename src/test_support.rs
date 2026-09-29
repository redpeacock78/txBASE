use std::sync::Mutex;

static CRASH_AT: Mutex<Option<String>> = Mutex::new(None);

pub(crate) fn arm_crash_at(point: &str) {
    *CRASH_AT.lock().unwrap() = Some(point.to_owned());
}

pub(crate) fn crash_at(point: &str) {
    if CRASH_AT.lock().unwrap().as_deref() == Some(point) {
        std::process::exit(86);
    }
}
