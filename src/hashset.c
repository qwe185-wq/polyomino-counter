/**
 * hashset.c — 哈希集合实现
 *
 * 开放寻址 + 线性探测。使用 splitmix64 变体哈希函数。
 * 负载因子超过 0.5 时自动扩容至 2 倍。
 */

#include "hashset.h"

/* ================================================================
 * 哈希函数 — splitmix64 变体（雪崩效应好，分布均匀）
 * ================================================================ */

static inline uint64_t hash_key(uint64_t key) {
    key = (key ^ (key >> 30)) * 0xBF58476D1CE4E5B9ULL;
    key = (key ^ (key >> 27)) * 0x94D049BB133111EBULL;
    key = key ^ (key >> 31);
    return key;
}

/* ================================================================
 * 公共接口
 * ================================================================ */

HashSet *hs_create(int initial_capacity) {
    HashSet *hs = (HashSet *)malloc(sizeof(HashSet));
    if (!hs) return NULL;

    /* 容量向上取整为 2 的幂 */
    int cap = 1;
    while (cap < initial_capacity) cap <<= 1;

    hs->keys = (mask_t *)calloc((size_t)cap, sizeof(mask_t));
    if (!hs->keys) { free(hs); return NULL; }

    hs->capacity = cap;
    hs->count = 0;
    return hs;
}

void hs_free(HashSet *hs) {
    if (hs) {
        free(hs->keys);
        free(hs);
    }
}

/* ---------- 内部：扩容 ---------- */

static bool hs_resize(HashSet *hs) {
    int old_cap = hs->capacity;
    mask_t *old_keys = hs->keys;

    int new_cap = old_cap * 2;
    mask_t *new_keys = (mask_t *)calloc((size_t)new_cap, sizeof(mask_t));
    if (!new_keys) return false;

    /* 重新哈希所有旧键 */
    for (int i = 0; i < old_cap; i++) {
        mask_t key = old_keys[i];
        if (key != 0) {
            uint64_t h = hash_key(key);
            int idx = (int)(h & (uint64_t)(new_cap - 1));
            while (new_keys[idx] != 0) {
                idx = (idx + 1) & (new_cap - 1);
            }
            new_keys[idx] = key;
        }
    }

    free(old_keys);
    hs->keys = new_keys;
    hs->capacity = new_cap;
    return true;
}

/* ---------- 插入 ---------- */

bool hs_insert(HashSet *hs, mask_t key) {
    if (key == 0) return false; /* 0 保留为空槽标记 */

    /* 负载因子 > 0.5 → 扩容 */
    if (hs->count * 2 >= hs->capacity) {
        if (!hs_resize(hs)) return false;
    }

    uint64_t h = hash_key(key);
    int idx = (int)(h & (uint64_t)(hs->capacity - 1));

    while (hs->keys[idx] != 0) {
        if (hs->keys[idx] == key) return false; /* 已存在 */
        idx = (idx + 1) & (hs->capacity - 1);
    }

    hs->keys[idx] = key;
    hs->count++;
    return true;
}

/* ---------- 查找 ---------- */

bool hs_contains(HashSet *hs, mask_t key) {
    if (key == 0 || hs->count == 0) return false;

    uint64_t h = hash_key(key);
    int idx = (int)(h & (uint64_t)(hs->capacity - 1));

    while (hs->keys[idx] != 0) {
        if (hs->keys[idx] == key) return true;
        idx = (idx + 1) & (hs->capacity - 1);
    }

    return false;
}

/* ---------- 计数 ---------- */

int hs_count(HashSet *hs) {
    return hs->count;
}
