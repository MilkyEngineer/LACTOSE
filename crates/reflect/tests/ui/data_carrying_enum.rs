use lactose_reflect::value;

#[value]
#[repr(u8)]
enum Held {
    Nothing = 0,
    Item(u8) = 1,
}

fn main() {}
