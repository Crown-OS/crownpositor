use crownos_config::schema::{Lock, LockSlotPosition, Theme};
use crownplugin_view::Tree;

use crate::{LockScene, Slot};

/// The scene for these settings, this theme, an optional theme layout and the
/// plugins' slots, each moved wherever the user placed it.
pub fn compose(
    config: &Lock,
    theme: &Theme,
    decoration: Option<Tree>,
    slots: Vec<(String, LockSlotPosition, Tree)>,
) -> LockScene {
    let slots = slots
        .into_iter()
        .map(|(id, position, tree)| {
            let (position, offset) = config
                .slot_placement(&id)
                .map_or((position, [0.0; 2]), |placed| {
                    (placed.position, placed.offset)
                });
            Slot {
                id,
                position,
                offset,
                tree: display_only(tree),
            }
        })
        .collect();
    LockScene {
        config: config.clone(),
        theme: theme.clone(),
        slots,
        decoration: decoration.map(display_only),
    }
}

fn display_only(tree: Tree) -> Tree {
    Tree {
        rev: tree.rev,
        root: tree.root.into_display_only(),
    }
}

#[cfg(test)]
mod tests {
    use crownos_config::schema::SlotPlacement;
    use crownplugin_view::{Node, NodeId};

    use super::*;

    #[test]
    fn the_users_placement_wins_and_inputs_are_stripped() {
        let config = Lock {
            slots: vec![SlotPlacement {
                widget: "plugin:a.b/c".into(),
                position: LockSlotPosition::Bottom,
                offset: [0.0, -12.0],
            }],
            ..Lock::default()
        };
        let tree = Tree {
            rev: 1,
            root: Node::Toggle {
                id: NodeId(1),
                label: "x".into(),
                on: true,
            },
        };
        let scene = compose(
            &config,
            &Theme::default(),
            None,
            vec![("plugin:a.b/c".into(), LockSlotPosition::BelowClock, tree)],
        );
        let slot = &scene.slots[0];
        assert_eq!(slot.position, LockSlotPosition::Bottom);
        assert_eq!(slot.offset, [0.0, -12.0]);
        assert_eq!(slot.tree.root, Node::Spacer);
    }
}
