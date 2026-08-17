use dioxus::prelude::*;
use dioxus_logger::tracing::Level;
use owls_ui::app::App;

fn main() {
    dioxus_logger::init(Level::DEBUG).expect("failed to init logger");
    launch(App);
}
