fn main() {
    let inst = wgpu::Instance::default();
    let ads = pollster::block_on(inst.enumerate_adapters(wgpu::Backends::all()));
    for a in ads {
        println!("{:?}", a.get_info());
    }
}
