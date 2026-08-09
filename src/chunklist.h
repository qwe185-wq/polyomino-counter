/**
 * chunklist.h — 分块数组
 *
 * 用固定大小块（64K 元素）的链表替代 realloc 大块连续内存。
 * 优势：
 *   1. 无 realloc 拷贝开销
 *   2. 不依赖大块连续内存（抗碎片化）
 *   3. O(1) 追加，O(total) 遍历
 */

#ifndef CHUNKLIST_H
#define CHUNKLIST_H

#include "common.h"
#include <stdlib.h>

/* 每块容纳 64K 个掩码（512KB/块） */
#define CHUNK_CAP 65536

typedef struct Chunk {
    mask_t data[CHUNK_CAP];
    int count;               /* 本块已用条目数 */
    struct Chunk *next;
} Chunk;

typedef struct {
    Chunk *head;
    Chunk *tail;
    int total;               /* 全部块的总条目数 */
} ChunkList;

/* ---------- 初始化 ---------- */
static inline void cl_init(ChunkList *cl) {
    cl->head = cl->tail = NULL;
    cl->total = 0;
}

/* ---------- 追加 ---------- */
static inline bool cl_add(ChunkList *cl, mask_t m) {
    if (!cl->tail || cl->tail->count >= CHUNK_CAP) {
        Chunk *c = (Chunk *)calloc(1, sizeof(Chunk));
        if (!c) return false;
        c->next = NULL;
        if (cl->tail) {
            cl->tail->next = c;
        } else {
            cl->head = c;
        }
        cl->tail = c;
    }
    cl->tail->data[cl->tail->count++] = m;
    cl->total++;
    return true;
}

/* ---------- 释放 ---------- */
static inline void cl_free(ChunkList *cl) {
    Chunk *c = cl->head;
    while (c) {
        Chunk *next = c->next;
        free(c);
        c = next;
    }
    cl->head = cl->tail = NULL;
    cl->total = 0;
}

/* ---------- 交换两个列表（O(1)，只交换指针） ---------- */
static inline void cl_swap(ChunkList *a, ChunkList *b) {
    ChunkList tmp = *a;
    *a = *b;
    *b = tmp;
}

#endif /* CHUNKLIST_H */
