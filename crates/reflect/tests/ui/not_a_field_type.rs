use lactose_reflect::component;

#[component(key = "ui::pointer_width", persistent)]
struct Index {
    #[field(id = 1)]
    row: usize,
}

#[component(key = "ui::bare_array", persistent)]
struct Cooldowns {
    #[field(id = 1)]
    ticks: [u32; 4],
}

fn main() {}
