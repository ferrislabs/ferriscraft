#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SheepKind {
    Lightning,
    Incendiary,
    Boarding,
    Healing,
    Seeker,
    Torpedo,
    Explosive,
    Splitting,
    Darkness,
    Frozen,
    Quake,
    Warp,
    Intergalactic,
    Fragment,
}

impl SheepKind {
    pub const ALL: [Self; 13] = [
        Self::Lightning,
        Self::Incendiary,
        Self::Boarding,
        Self::Healing,
        Self::Seeker,
        Self::Torpedo,
        Self::Explosive,
        Self::Splitting,
        Self::Darkness,
        Self::Frozen,
        Self::Quake,
        Self::Warp,
        Self::Intergalactic,
    ];
}
