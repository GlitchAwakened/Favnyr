//! Native discovery is confined to a temporary apartment; UI models stay local.

use super::*;
use crate::shellmenu::{DocumentChoice, discover_documents};
use std::sync::mpsc;

struct Delivery {
    generation: u64,
    result: Result<Vec<DocumentChoice>, String>,
}

pub(super) struct Discovery {
    cache: Rc<Cache>,
    waiting: RefCell<Option<Location>>,
    sender: mpsc::Sender<Delivery>,
    window: slint::Weak<MainWindow>,
}

impl Discovery {
    pub fn install(
        window: &MainWindow,
        state: &AppState,
        cache: Rc<Cache>,
        context: Rc<RefCell<Option<Context>>>,
    ) -> Rc<Self> {
        let (sender, receiver) = mpsc::channel::<Delivery>();
        let discovery = Rc::new(Self {
            cache,
            waiting: RefCell::new(None),
            sender,
            window: window.as_weak(),
        });
        let loader = discovery.clone();
        let st = state.clone();
        window.on_document_new_delivered(move || {
            while let Ok(delivery) = receiver.try_recv() {
                let Some(window) = loader.window.upgrade() else {
                    return;
                };
                loader.deliver(&window, &context, delivery, |location| {
                    !shutdown::stopping(&st) && location.is_current(&st)
                });
            }
        });
        discovery
    }

    fn deliver(
        &self,
        window: &MainWindow,
        context: &RefCell<Option<Context>>,
        delivery: Delivery,
        is_current: impl FnOnce(&Location) -> bool,
    ) {
        let catalog = match delivery.result {
            Ok(choices) => Some(Catalog::from_choices(choices)),
            Err(error) => {
                debug!(%error, "registered document discovery unavailable");
                None
            }
        };
        let accepted = self.cache.finish(delivery.generation, catalog);
        let location = self.waiting.borrow_mut().take();
        let Some(location) =
            location.filter(|location| window.get_new_menu_open() && is_current(location))
        else {
            return;
        };
        if let Some(catalog) = self.cache.get() {
            let rows = catalog.rows.clone();
            *context.borrow_mut() = Some(Context { location, catalog });
            window.set_new_documents(rows);
        } else if !accepted {
            // A settings change invalidated the old request. Only an
            // open, still relevant menu can request its replacement.
            self.request(location);
        }
        // A failure is retried on the next menu opening, never in a loop.
    }

    pub fn release(&self) {
        self.waiting.borrow_mut().take();
    }

    pub fn request(&self, location: Location) {
        let folder = location.folder.clone();
        *self.waiting.borrow_mut() = Some(location);
        let Some(generation) = self.cache.begin() else {
            return;
        };
        let sender = self.sender.clone();
        let weak = self.window.clone();
        let worker = std::thread::Builder::new()
            .name("document-discovery".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(|| discover_documents(&folder))
                    .map_err(|_| "document discovery panicked".to_string())
                    .and_then(|result| result.map_err(|error| error.to_string()));
                // Native objects and their apartment have already been released.
                if sender.send(Delivery { generation, result }).is_ok() {
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(window) = weak.upgrade() {
                            window.invoke_document_new_delivered();
                        }
                    });
                }
            });
        if let Err(error) = worker {
            self.cache.finish(generation, None);
            self.release();
            debug!(%error, "registered document discovery could not start");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn location(folder: &str) -> Location {
        Location {
            book: Default::default(),
            tab: Tab::new(folder.into(), false).id,
            folder: folder.into(),
        }
    }

    fn delivery(generation: u64) -> Delivery {
        Delivery {
            generation,
            result: Ok(vec![DocumentChoice {
                extension: ".bin".into(),
                label: "File01".into(),
                icon: None,
            }]),
        }
    }

    #[test]
    fn late_results_fill_cache_but_only_attach_to_the_current_open_menu() {
        crate::test_support::run(|| {
            for scenario in 0..4 {
                let window = MainWindow::new().unwrap();
                let (sender, _) = mpsc::channel();
                let loader = Discovery {
                    cache: Rc::new(Cache::default()),
                    waiting: RefCell::new(Some(location("folder01"))),
                    sender,
                    window: window.as_weak(),
                };
                let context = RefCell::new(None);
                let request = loader.cache.begin().unwrap();
                window.set_new_menu_open(scenario != 0);
                match scenario {
                    0 => loader.release(),
                    1 => {
                        loader.release();
                        *loader.waiting.borrow_mut() = Some(location("folder02"));
                    }
                    2 => {}
                    3 => {
                        loader.cache.clear();
                        loader.release();
                    }
                    _ => unreachable!(),
                }
                loader.deliver(&window, &context, delivery(request), |location| {
                    location.folder == std::path::Path::new("folder02")
                });
                assert_eq!(loader.cache.get().is_some(), scenario != 3);
                assert_eq!(
                    window.get_new_documents().row_count(),
                    usize::from(scenario == 1)
                );
                assert_eq!(context.borrow().is_some(), scenario == 1);
                if let Some(context) = context.borrow().as_ref() {
                    assert_eq!(context.location.folder, std::path::Path::new("folder02"));
                }
                assert!(loader.waiting.borrow().is_none());
            }
        });
    }

    #[test]
    fn failed_discovery_releases_the_request_without_automatic_retries() {
        crate::test_support::run(|| {
            let window = MainWindow::new().unwrap();
            window.set_new_menu_open(true);
            let (sender, _) = mpsc::channel();
            let loader = Discovery {
                cache: Rc::new(Cache::default()),
                waiting: RefCell::new(Some(location("folder01"))),
                sender,
                window: window.as_weak(),
            };
            let generation = loader.cache.begin().unwrap();
            loader.deliver(
                &window,
                &RefCell::new(None),
                Delivery {
                    generation,
                    result: Err("unavailable".into()),
                },
                |_| true,
            );
            assert!(loader.waiting.borrow().is_none());
            assert!(loader.cache.get().is_none());
            assert!(loader.cache.begin().is_some());
            assert_eq!(window.get_new_documents().row_count(), 0);
        });
    }
}
