use lactose_reflect::value;

#[value]
#[repr(u8)]
#[rewrite_macros::align16]
pub enum Mode {
    Off = 0,
    On = 1,
}

fn main() {}
