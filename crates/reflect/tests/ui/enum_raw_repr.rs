use lactose_reflect::value;

#[value]
#[repr(u8)]
#[r#repr(align(4))]
pub enum Mode {
    Off = 0,
    On = 1,
}

fn main() {}
