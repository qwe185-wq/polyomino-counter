/**
 * hashset.h — 哈希集合接口
 *
 * 开放寻址 + 线性探测。splitmix64 变体哈希。
 * 多线程策略: 读取无锁，写入由调用方同步（omp critical）
 */

#ifndef HASHSET_H
#define HASHSET_H

#include "common.h"

typedef struct {
    mask_t *keys;
    int capacity;
    int count;
} HashSet;

HashSet *hs_create(int initial_capacity);
void hs_free(HashSet *hs);

/* 插入（可扩容）。并行区外或临界区内使用 */
bool hs_insert(HashSet *hs, mask_t key);

/* 无锁读取 — 安全前提: 并行期内不扩容 */
bool hs_contains(HashSet *hs, mask_t key);

int hs_count(HashSet *hs);

#endif
