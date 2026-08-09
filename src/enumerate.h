/**
 * enumerate.h — 多联骨牌枚举与房间计数接口
 */

#ifndef ENUMERATE_H
#define ENUMERATE_H

#include "common.h"

/**
 * 主枚举入口：对 n = 1..max_n 分别统计
 *
 * 返回 RoomCount 数组（长度 = max_n），调用者负责 free。
 * 失败返回 NULL。
 */
RoomCount *enumerate_all(int max_n, int *out_count);

#endif /* ENUMERATE_H */
