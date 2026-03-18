use std::panic::{self, AssertUnwindSafe};
use std::sync::mpsc::{self, Sender};
use std::sync::OnceLock;
use std::thread;

use gtk4::glib;
use gtk4::prelude::*;

type GtkTask = Box<dyn FnOnce() + Send + 'static>;

fn gtk_test_sender() -> &'static Sender<GtkTask> {
    static SENDER: OnceLock<Sender<GtkTask>> = OnceLock::new();
    SENDER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<GtkTask>();
        thread::Builder::new()
            .name("cmux-gtk-test".into())
            .spawn(move || {
                gtk4::init().expect("GTK should initialize for Linux UI tests");

                while let Ok(task) = rx.recv() {
                    task();
                    flush_main_loop();
                }
            })
            .expect("GTK test thread should start");
        tx
    })
}

pub(crate) fn run_on_gtk_thread<R, F>(f: F) -> R
where
    R: Send + 'static,
    F: FnOnce() -> R + Send + 'static,
{
    let (result_tx, result_rx) = mpsc::sync_channel(1);
    gtk_test_sender()
        .send(Box::new(move || {
            let result = panic::catch_unwind(AssertUnwindSafe(f));
            result_tx
                .send(result)
                .expect("GTK test result should be delivered");
        }))
        .expect("GTK test task should be scheduled");

    match result_rx
        .recv()
        .expect("GTK test result should be received")
    {
        Ok(result) => result,
        Err(payload) => panic::resume_unwind(payload),
    }
}

pub(crate) fn flush_main_loop() {
    let context = glib::MainContext::default();
    for _ in 0..8 {
        while context.pending() {
            let _ = context.iteration(false);
        }
    }
}

pub(crate) fn mount_widget<W: IsA<gtk4::Widget>>(widget: &W) -> gtk4::Window {
    let window = gtk4::Window::new();
    window.set_default_size(900, 700);
    window.set_child(Some(widget));
    window.present();
    flush_main_loop();
    window
}

pub(crate) fn close_window(window: gtk4::Window) {
    window.close();
    flush_main_loop();
}

pub(crate) fn find_descendant<T>(root: &gtk4::Widget) -> Option<T>
where
    T: IsA<gtk4::Widget> + Clone + 'static,
{
    if let Ok(found) = root.clone().downcast::<T>() {
        return Some(found);
    }

    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Some(found) = find_descendant::<T>(&widget) {
            return Some(found);
        }
        child = widget.next_sibling();
    }

    None
}
