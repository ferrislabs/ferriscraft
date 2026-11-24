use ferriscraft_towerwars::game::Layout;
use ferriscraft_towerwars::map::Map;

fn main() {
    let layout = Layout::from_map(&Map::test_map());
    println!(
        "{} blocs, {} chunks",
        layout.blocks().count(),
        layout.chunks().len()
    );
}
