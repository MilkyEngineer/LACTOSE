use lactose_reflect::component;

#[component(key = "ui::duplicate_field_id", persistent)]
struct Health {
    #[field(id = 1)]
    current: u32,
    #[field(id = 1)]
    max: u32,
}

fn main() {}
