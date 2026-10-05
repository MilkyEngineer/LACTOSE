use lactose_reflect::component;

#[component(key = "ui::padding_type", persistent)]
struct Padded {
    #[field(id = 1)]
    count: u32,
    #[field(id = 2)]
    flag: bool,
    #[field(padding)]
    _pad: [u8; 3],
}

fn main() {}
