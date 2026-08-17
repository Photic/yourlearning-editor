use dioxus::prelude::*;
use dioxus_logger::tracing::Level;
use owls_ui::app::App;

fn main() {
    console_error_panic_hook::set_once();
    dioxus_logger::init(Level::DEBUG).expect("failed to init logger");
    launch(App);
}
