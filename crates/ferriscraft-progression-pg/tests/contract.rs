mod common;

use ferriscraft_progression::MemoryProgressionRepository;

#[tokio::test]
async fn memory_passes_the_contract() {
    common::contract(MemoryProgressionRepository::default()).await;
}
