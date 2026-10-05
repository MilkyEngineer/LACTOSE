use lactose_reflect::component;

#[component(key = "ui::padding_between", persistent)]
struct Between {
    #[field(id = 1)]
    flag: bool,
    #[field(id = 2)]
    count: u32,
}

#[component(key = "ui::padding_after", persistent)]
struct After {
    #[field(id = 1)]
    count: u32,
    #[field(id = 2)]
    flag: bool,
}

fn main() {}
