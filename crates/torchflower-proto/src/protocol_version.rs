#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[repr(u32)]
pub enum ProtocolVersion {
    #[default]
    V1_21_100 = 766,
    V1_21_130 = 776,
}
