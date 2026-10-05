use lactose_reflect::component;

#[component(key = "ui::generic", persistent)]
struct Slot<T> {
    #[field(id = 1)]
    value: T,
}

fn main() {}
