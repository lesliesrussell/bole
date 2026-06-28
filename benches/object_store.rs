// bole-2q8
use bole::store::{memory::MemoryBackend, ObjectStore};
use bytes::Bytes;
use criterion::{criterion_group, criterion_main, Criterion};

fn make_store() -> ObjectStore {
    ObjectStore::new(MemoryBackend::new())
}

fn bench_put_blob_cold(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let payload = Bytes::from(vec![42u8; 1024]);
    c.bench_function("put_blob_cold", |b| {
        b.iter(|| {
            let store = make_store();
            let p = payload.clone();
            rt.block_on(async move { store.put_blob(p).await.unwrap() })
        })
    });
}

fn bench_put_blob_dedup(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let store = make_store();
    let payload = Bytes::from(vec![42u8; 1024]);
    rt.block_on(async { store.put_blob(payload.clone()).await.unwrap() });
    c.bench_function("put_blob_dedup", |b| {
        b.iter(|| {
            let p = payload.clone();
            rt.block_on(async { store.put_blob(p).await.unwrap() })
        })
    });
}

fn bench_get_blob(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let store = make_store();
    let payload = Bytes::from(vec![42u8; 1024]);
    let id = rt.block_on(async { store.put_blob(payload).await.unwrap() });
    c.bench_function("get_blob", |b| {
        b.iter(|| rt.block_on(async { store.get(&id).await.unwrap() }))
    });
}

criterion_group!(benches, bench_put_blob_cold, bench_put_blob_dedup, bench_get_blob);
criterion_main!(benches);
