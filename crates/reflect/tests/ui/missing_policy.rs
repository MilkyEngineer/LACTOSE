use lactose_reflect::component;

#[component(key = "ui::missing_policy")]
struct Health {
    #[field(id = 1)]
    current: u32,
}

fn main() {}
