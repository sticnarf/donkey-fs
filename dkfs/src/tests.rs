use device::*;
use *;

#[test]
fn format() {
    let mut mem = vec![0; 268435456]; // 256MB
    {
        let mem = Box::new(Memory::new(&mut mem[..]));
        ::format(mem, FormatOptions::default()).ok();
    }
}
