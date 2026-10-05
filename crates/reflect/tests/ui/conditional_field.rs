use lactose_reflect::component;

#[component(key = "ui::conditional_field", persistent)]
struct Health {
    #[field(id = 1)]
    current: u32,
    #[cfg(any())]
    #[field(id = 2)]
    extra: u32,
}

fn main() {}
