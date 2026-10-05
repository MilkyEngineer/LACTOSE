use lactose_reflect::component;

#[component(key = "ui::retired_field_id", persistent, retired = [2])]
struct Health {
    #[field(id = 1)]
    current: u32,
    #[field(id = 2)]
    max: u32,
}

fn main() {}
