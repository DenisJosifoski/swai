use gtk::prelude::*;
use gtk4 as gtk;

/// Show an error message dialog.
pub fn show_error<P: gtk::prelude::IsA<gtk::Window>>(
    parent: Option<&P>,
    message: &str,
    title: &str,
) {
    let error_dialog = gtk::MessageDialog::new(
        parent,
        gtk::DialogFlags::MODAL,
        gtk::MessageType::Error,
        gtk::ButtonsType::Close,
        message,
    );
    error_dialog.set_title(Some(title));
    error_dialog.connect_response(|d, _| d.destroy());
    error_dialog.present();
}

/// Open the script file in the system's default editor.
pub fn launch_script_editor(script_path: &std::path::PathBuf) {
    let uri = gio::File::for_path(script_path).uri();
    tracing::debug!("Launching default editor for {}", uri);
    let _ = gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>);
}
