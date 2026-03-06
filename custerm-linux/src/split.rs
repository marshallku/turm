use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;

use custerm_core::config::CustermConfig;

use crate::panel::Panel;
use crate::terminal::TerminalPanel;

pub enum SplitNode {
    Leaf {
        panel: Rc<TerminalPanel>,
    },
    Branch {
        paned: gtk4::Paned,
        first: Rc<RefCell<SplitNode>>,
        second: Rc<RefCell<SplitNode>>,
    },
}

impl SplitNode {
    pub fn widget(&self) -> &gtk4::Widget {
        match self {
            SplitNode::Leaf { panel } => panel.widget(),
            SplitNode::Branch { paned, .. } => paned.upcast_ref(),
        }
    }

    pub fn collect_panels(&self, out: &mut Vec<Rc<TerminalPanel>>) {
        match self {
            SplitNode::Leaf { panel } => out.push(panel.clone()),
            SplitNode::Branch { first, second, .. } => {
                first.borrow().collect_panels(out);
                second.borrow().collect_panels(out);
            }
        }
    }

    pub fn panel_count(&self) -> usize {
        match self {
            SplitNode::Leaf { .. } => 1,
            SplitNode::Branch { first, second, .. } => {
                first.borrow().panel_count() + second.borrow().panel_count()
            }
        }
    }

    /// Find the parent node that contains `target` panel, and which side (first/second)
    pub fn find_parent(
        node: &Rc<RefCell<SplitNode>>,
        target: &Rc<TerminalPanel>,
    ) -> Option<(Rc<RefCell<SplitNode>>, ChildSide)> {
        let borrowed = node.borrow();
        if let SplitNode::Branch { first, second, .. } = &*borrowed {
            // Check if first child is the target
            if let SplitNode::Leaf { panel } = &*first.borrow() {
                if Rc::ptr_eq(panel, target) {
                    return Some((node.clone(), ChildSide::First));
                }
            }
            // Check if second child is the target
            if let SplitNode::Leaf { panel } = &*second.borrow() {
                if Rc::ptr_eq(panel, target) {
                    return Some((node.clone(), ChildSide::Second));
                }
            }
            // Recurse
            if let Some(found) = Self::find_parent(first, target) {
                return Some(found);
            }
            if let Some(found) = Self::find_parent(second, target) {
                return Some(found);
            }
        }
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChildSide {
    First,
    Second,
}

/// A tab's content: a stable container + split tree
pub struct TabContent {
    pub container: gtk4::Box,
    pub root: Rc<RefCell<SplitNode>>,
}

impl TabContent {
    pub fn new(panel: Rc<TerminalPanel>) -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);
        container.append(panel.widget());

        let root = Rc::new(RefCell::new(SplitNode::Leaf { panel }));

        Self { container, root }
    }

    /// Split the focused panel in the given orientation
    pub fn split(
        &self,
        focused: &Rc<TerminalPanel>,
        orientation: gtk4::Orientation,
        config: &CustermConfig,
        on_exit: impl Fn() + 'static,
    ) -> Rc<TerminalPanel> {
        let new_panel = Rc::new(TerminalPanel::new(config, on_exit));

        // Apply background to new panel
        if let Some(ref path) = config.background.image {
            let p = std::path::Path::new(path);
            if p.exists() {
                new_panel.set_background(p);
            }
        }

        let paned = gtk4::Paned::new(orientation);
        paned.set_hexpand(true);
        paned.set_vexpand(true);
        paned.set_wide_handle(true);

        let old_widget = focused.widget().clone();
        let new_leaf = Rc::new(RefCell::new(SplitNode::Leaf {
            panel: new_panel.clone(),
        }));

        // Check if focused is the root
        let is_root = {
            let root = self.root.borrow();
            if let SplitNode::Leaf { panel } = &*root {
                Rc::ptr_eq(panel, focused)
            } else {
                false
            }
        };

        if is_root {
            // Remove old widget from container
            self.container.remove(&old_widget);

            // Build paned
            paned.set_start_child(Some(&old_widget));
            paned.set_end_child(Some(new_panel.widget()));

            let old_leaf = Rc::new(RefCell::new(SplitNode::Leaf {
                panel: focused.clone(),
            }));

            *self.root.borrow_mut() = SplitNode::Branch {
                paned: paned.clone(),
                first: old_leaf,
                second: new_leaf,
            };

            self.container.append(paned.upcast_ref::<gtk4::Widget>());
        } else {
            // Find parent branch
            if let Some((parent_node, side)) = SplitNode::find_parent(&self.root, focused) {
                let mut parent = parent_node.borrow_mut();
                if let SplitNode::Branch {
                    paned: parent_paned,
                    first,
                    second,
                } = &mut *parent
                {
                    // Remove old widget from parent paned
                    match side {
                        ChildSide::First => parent_paned.set_start_child(gtk4::Widget::NONE),
                        ChildSide::Second => parent_paned.set_end_child(gtk4::Widget::NONE),
                    }

                    paned.set_start_child(Some(&old_widget));
                    paned.set_end_child(Some(new_panel.widget()));

                    let old_leaf = Rc::new(RefCell::new(SplitNode::Leaf {
                        panel: focused.clone(),
                    }));

                    let new_branch = Rc::new(RefCell::new(SplitNode::Branch {
                        paned: paned.clone(),
                        first: old_leaf,
                        second: new_leaf,
                    }));

                    match side {
                        ChildSide::First => {
                            parent_paned.set_start_child(Some(paned.upcast_ref::<gtk4::Widget>()));
                            *first = new_branch;
                        }
                        ChildSide::Second => {
                            parent_paned.set_end_child(Some(paned.upcast_ref::<gtk4::Widget>()));
                            *second = new_branch;
                        }
                    }
                }
            }
        }

        // Set paned position to 50%
        paned.connect_realize(move |p| {
            let size = match orientation {
                gtk4::Orientation::Horizontal => p.width(),
                _ => p.height(),
            };
            p.set_position(size / 2);
        });

        new_panel
    }

    /// Close a panel within the split tree. Returns true if the tab itself should close.
    pub fn close_panel(&self, target: &Rc<TerminalPanel>) -> bool {
        // If root is a leaf, closing it means closing the tab
        {
            let root = self.root.borrow();
            if let SplitNode::Leaf { panel } = &*root {
                if Rc::ptr_eq(panel, target) {
                    return true;
                }
            }
        }

        // Find parent and replace it with the sibling
        if let Some((parent_node, side)) = SplitNode::find_parent(&self.root, target) {
            let sibling_node;
            let sibling_widget;

            {
                let parent = parent_node.borrow();
                if let SplitNode::Branch {
                    paned: _,
                    first,
                    second,
                } = &*parent
                {
                    let sibling = match side {
                        ChildSide::First => second,
                        ChildSide::Second => first,
                    };
                    sibling_widget = sibling.borrow().widget().clone();
                    sibling_node = sibling.clone();
                } else {
                    return false;
                }
            }

            // Check if parent_node is the root
            let parent_is_root = Rc::ptr_eq(&parent_node, &self.root);

            if parent_is_root {
                // Remove the paned from container
                let old_widget = self.root.borrow().widget().clone();
                self.container.remove(&old_widget);

                // Detach sibling from old paned
                {
                    let parent = parent_node.borrow();
                    if let SplitNode::Branch { paned, .. } = &*parent {
                        paned.set_start_child(gtk4::Widget::NONE);
                        paned.set_end_child(gtk4::Widget::NONE);
                    }
                }

                // Replace root with sibling
                let new_root = sibling_node.borrow().clone_node();
                self.container.append(new_root.widget());
                *self.root.borrow_mut() = new_root;
            } else {
                // Find grandparent
                if let Some((grandparent_node, parent_side)) =
                    SplitNode::find_branch_parent(&self.root, &parent_node)
                {
                    let mut grandparent = grandparent_node.borrow_mut();
                    if let SplitNode::Branch {
                        paned: gp_paned,
                        first,
                        second,
                    } = &mut *grandparent
                    {
                        // Detach sibling from old paned
                        {
                            let parent = parent_node.borrow();
                            if let SplitNode::Branch { paned, .. } = &*parent {
                                paned.set_start_child(gtk4::Widget::NONE);
                                paned.set_end_child(gtk4::Widget::NONE);
                            }
                        }

                        // Detach old parent paned from grandparent
                        match parent_side {
                            ChildSide::First => {
                                gp_paned.set_start_child(gtk4::Widget::NONE);
                                gp_paned.set_start_child(Some(&sibling_widget));
                                *first = sibling_node;
                            }
                            ChildSide::Second => {
                                gp_paned.set_end_child(gtk4::Widget::NONE);
                                gp_paned.set_end_child(Some(&sibling_widget));
                                *second = sibling_node;
                            }
                        }
                    }
                }
            }
        }

        false
    }
}

impl SplitNode {
    /// Find the parent branch that contains a child branch node (not leaf)
    fn find_branch_parent(
        node: &Rc<RefCell<SplitNode>>,
        target: &Rc<RefCell<SplitNode>>,
    ) -> Option<(Rc<RefCell<SplitNode>>, ChildSide)> {
        let borrowed = node.borrow();
        if let SplitNode::Branch { first, second, .. } = &*borrowed {
            if Rc::ptr_eq(first, target) {
                return Some((node.clone(), ChildSide::First));
            }
            if Rc::ptr_eq(second, target) {
                return Some((node.clone(), ChildSide::Second));
            }
            if let Some(found) = Self::find_branch_parent(first, target) {
                return Some(found);
            }
            if let Some(found) = Self::find_branch_parent(second, target) {
                return Some(found);
            }
        }
        None
    }

    fn clone_node(&self) -> SplitNode {
        match self {
            SplitNode::Leaf { panel } => SplitNode::Leaf {
                panel: panel.clone(),
            },
            SplitNode::Branch {
                paned,
                first,
                second,
            } => SplitNode::Branch {
                paned: paned.clone(),
                first: first.clone(),
                second: second.clone(),
            },
        }
    }
}
