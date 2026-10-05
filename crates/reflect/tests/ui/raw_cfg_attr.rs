use lactose_reflect::component;

#[component(key = "ui::raw_cfg_attr", persistent)]
#[r#cfg_attr(all(), r#repr(align(8)))]
pub struct Aligned {
    #[field(id = 1)]
    pub a: u32,
    #[field(id = 2)]
    pub b: u32,
}

fn main() {}
