use std::collections::HashMap;

/// `org.freedesktop.ScreenSaver` holds by cookie, each with the unique bus
/// name that took it: only that name may let it go, and they all go when it
/// leaves the bus, so a crashed browser cannot keep the screen on forever.
#[derive(Debug, Default)]
pub struct ScreenSaverHolds {
    owners: HashMap<u32, String>,
}

impl ScreenSaverHolds {
    pub fn hold(&mut self, cookie: u32, owner: String) {
        self.owners.insert(cookie, owner);
    }

    pub fn release(&mut self, cookie: u32, owner: &str) {
        if self.owners.get(&cookie).is_some_and(|held| held == owner) {
            self.owners.remove(&cookie);
        }
    }

    pub fn release_owner(&mut self, owner: &str) {
        self.owners.retain(|_, held| held != owner);
    }

    pub fn is_empty(&self) -> bool {
        self.owners.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_holder_lets_go() {
        let mut holds = ScreenSaverHolds::default();
        holds.hold(1, ":1.64".into());
        holds.release(1, ":1.70");
        assert!(!holds.is_empty());
        holds.release(1, ":1.64");
        assert!(holds.is_empty());
    }

    #[test]
    fn leaving_the_bus_lets_go_of_everything_held() {
        let mut holds = ScreenSaverHolds::default();
        holds.hold(1, ":1.64".into());
        holds.hold(2, ":1.64".into());
        holds.hold(3, ":1.80".into());
        holds.release_owner(":1.64");
        assert!(!holds.is_empty());
        holds.release_owner(":1.80");
        assert!(holds.is_empty());
    }
}
