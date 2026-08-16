use gtk::prelude::*;
use gtk::{Box as GtkBox, Label, Orientation, ScrolledWindow};
use gtk4 as gtk;

use crate::model_card::{CardState, ModelCard};

pub fn build_cards_container(cards_box: &GtkBox) -> ScrolledWindow {
    let scrolled = ScrolledWindow::new();
    scrolled.set_hexpand(true);
    scrolled.set_vexpand(true);
    scrolled.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
    scrolled.set_child(Some(cards_box));
    scrolled
}

pub fn reorder_card_container(cards: &[ModelCard]) {
    for card in cards {
        if matches!(
            card.state(),
            CardState::Ready | CardState::Starting | CardState::Loading
        ) {
            card.widget.set_css_classes(&["card-active"]);
        } else {
            card.widget.set_css_classes(&["card"]);
        }
    }
}

pub fn build_footer_bar(proxy_port: u16) -> (gtk::Box, gtk::Label, gtk::Label) {
    let footer = GtkBox::new(Orientation::Horizontal, 0);
    footer.set_css_classes(&["toolbar"]);
    footer.set_margin_start(12);
    footer.set_margin_end(12);
    footer.set_margin_bottom(6);

    let proxy_label = Label::new(Some(&format!("Proxy: 127.0.0.1:{proxy_port}")));
    proxy_label.set_css_classes(&["dim-label"]);
    proxy_label.set_halign(gtk::Align::Start);
    footer.append(&proxy_label);

    let spacer = Label::new(Some(""));
    spacer.set_hexpand(true);
    footer.append(&spacer);

    let model_label = Label::new(Some(&format!("SWAI v{}", env!("CARGO_PKG_VERSION"))));
    model_label.set_css_classes(&["dim-label"]);
    model_label.set_halign(gtk::Align::End);
    footer.append(&model_label);

    (footer, proxy_label, model_label)
}
