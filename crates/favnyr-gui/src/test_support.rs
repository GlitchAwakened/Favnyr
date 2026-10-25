//! Headless UI tests share the same backend setup and stack allowance.

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use std::{cell::RefCell, rc::Rc};

type TestWindows = Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>;

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
