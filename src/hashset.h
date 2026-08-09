/**
 * hashset.h — 哈希集合接口
 *
 * 使用开放寻址 + 线性探测，key 为 uint64_t 规范化位图。
 * 用于多联骨牌枚举过程中的去重。
 */

#ifndef HASHSET_H
#define HASHSET_H

#include "common.h"

typedef struct {
    mask_t *keys;      /* 键数组，0 表示空槽 */
    int capacity;      /* 总容量（2的幂） */
    int count;         /* 当前元素数 */
} HashSet;

/* 创建哈希集合。initial_capacity 会自动向上取整为 2 的幂 */
HashSet *hs_create(int initial_capacity);

/* 释放 */
void hs_free(HashSet *hs);

/* 插入。key 已存在则返回 false，否则返回 true */
bool hs_insert(HashSet *hs, mask_t key);

/* 查找。存在返回 true */
bool hs_contains(HashSet *hs, mask_t key);

/* 元素数量 */
int hs_count(HashSet *hs);

#endif /* HASHSET_H */
