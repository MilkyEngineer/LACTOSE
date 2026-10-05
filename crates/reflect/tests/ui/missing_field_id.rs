use lactose_reflect::component;

#[component(key = "ui::missing_field_id", persistent)]
struct Health {
    #[field(id = 1)]
    current: u32,
    max: u32,
}

fn main() {}
