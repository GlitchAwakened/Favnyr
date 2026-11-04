//! Headless UI tests share the same backend setup and stack allowance.

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use std::{cell::RefCell, rc::Rc};

type TestWindows = Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>;

/// Only an exclusively created workspace folder may be removed by a test.
pub(crate) struct TestFolder(pub(crate) std::path::PathBuf);

impl TestFolder {
    pub(crate) fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .unwrap()
            .join("target");
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = root.join(format!(
            "test-folder-{}-{time}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub(crate) fn run(test: impl FnOnce() + Send + 'static) {
    run_with_windows(move |_| test());
}

pub(crate) fn run_with_windows(test: impl FnOnce(TestWindows) + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            struct TestPlatform(TestWindows);
            impl slint::platform::Platform for TestPlatform {
                fn create_window_adapter(
                    &self,
                ) -> Result<std::rc::Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError>
                {
                    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
                    self.0.borrow_mut().push(window.clone());
                    Ok(window)
                }
            }
            let windows = TestWindows::default();
            slint::platform::set_platform(Box::new(TestPlatform(windows.clone()))).unwrap();
            test(windows);
        })
        .unwrap()
        .join()
        .unwrap();
}
