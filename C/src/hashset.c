/**
 * hashset.c — 哈希集合实现
 *
 * 开放寻址 + 线性探测。splitmix64 变体哈希。
 * 多线程安全: 读取无锁 + 写入由 omp critical 保护
 */

#include "hashset.h"

static inline uint64_t hash_key(uint64_t key) {
    key = (key ^ (key >> 30)) * 0xBF58476D1CE4E5B9ULL;
    key = (key ^ (key >> 27)) * 0x94D049BB133111EBULL;
    key = key ^ (key >> 31);
    return key;
}

HashSet *hs_create(int initial_capacity) {
    HashSet *hs = (HashSet *)malloc(sizeof(HashSet));
    if (!hs) return NULL;
    int cap = 1;
    while (cap < initial_capacity) cap <<= 1;
    hs->keys = (mask_t *)calloc((size_t)cap, sizeof(mask_t));
    if (!hs->keys) { free(hs); return NULL; }
    hs->capacity = cap;
    hs->count = 0;
    return hs;
}

void hs_free(HashSet *hs) {
    if (!hs) return;
    free(hs->keys);
    free(hs);
}

static bool hs_resize(HashSet *hs) {
    int old_cap = hs->capacity;
    mask_t *old_keys = hs->keys;
    int new_cap = old_cap * 2;
    mask_t *new_keys = (mask_t *)calloc((size_t)new_cap, sizeof(mask_t));
    if (!new_keys) return false;
    for (int i = 0; i < old_cap; i++) {
        mask_t key = old_keys[i];
        if (key != 0) {
            uint64_t h = hash_key(key);
            int idx = (int)(h & (uint64_t)(new_cap - 1));
            while (new_keys[idx] != 0)
                idx = (idx + 1) & (new_cap - 1);
            new_keys[idx] = key;
        }
    }
    free(old_keys);
    hs->keys = new_keys;
    hs->capacity = new_cap;
    return true;
}

bool hs_insert(HashSet *hs, mask_t key) {
    if (key == 0) return false;
    if (hs->count * 2 >= hs->capacity) {
        if (!hs_resize(hs)) return false;
    }
    uint64_t h = hash_key(key);
    int idx = (int)(h & (uint64_t)(hs->capacity - 1));
    while (hs->keys[idx] != 0) {
        if (hs->keys[idx] == key) return false;
        idx = (idx + 1) & (hs->capacity - 1);
    }
    hs->keys[idx] = key;
    hs->count++;
    return true;
}

bool hs_contains(HashSet *hs, mask_t key) {
    if (key == 0) return false;
    uint64_t h = hash_key(key);
    int idx = (int)(h & (uint64_t)(hs->capacity - 1));
    while (hs->keys[idx] != 0) {
        if (hs->keys[idx] == key) return true;
        idx = (idx + 1) & (hs->capacity - 1);
    }
    return false;
}

int hs_count(HashSet *hs) {
    return hs->count;
}
