pub trait Panel {
    fn widget(&self) -> &gtk4::Widget;
    fn title(&self) -> String;
    fn panel_type(&self) -> &str;
    fn grab_focus(&self);
}
