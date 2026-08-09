/**
 * timer.h — 函数级计时 Profiler（运行时开关）
 *
 * 用法：
 *   ./room-count.exe 5          # 正常模式，零额外开销
 *   ./room-count.exe 5 --time   # 开启计时，结束时输出各函数耗时占比
 */

#ifndef TIMER_H
#define TIMER_H

#include <time.h>
#include <stdio.h>

/* ================================================================
 * 计时器 ID 枚举
 * ================================================================ */

enum {
    TIMER_TOTAL = 0,       /* 程序总耗时 */
    TIMER_EXTRACT,         /* 掩码 → 坐标提取 */
    TIMER_FRONTIER,        /* 前沿计算 */
    TIMER_GROW,            /* 扩展格子 + 构造新掩码 */
    TIMER_CANONICAL,       /* One-sided 规范化（4 旋转 + 规范化） */
    TIMER_HOLE,            /* Flood fill 洞检测 */
    TIMER_HASHSET,         /* 哈希集合插入（含扩容） */
    TIMER_COUNT            /* 计时器总数 */
};

/* ================================================================
 * 数据结构
 * ================================================================ */

typedef struct {
    const char *name;      /* 计时器名称 */
    double total_sec;      /* 累计秒数 */
    clock_t start_tick;    /* 最近一次 START 的快照 */
    int call_count;        /* 调用次数 */
} TimerSlot;

/* ================================================================
 * 全局变量（main.c 中定义）
 * ================================================================ */

extern TimerSlot g_timers[TIMER_COUNT];
extern int g_timing_enabled;

/* ================================================================
 * 宏（开销极小：单次布尔分支 + clock() 调用）
 * ================================================================ */

#define TIMER_INIT()  timer_init()

#define TIMER_START(id)  do { \
    if (g_timing_enabled) g_timers[(id)].start_tick = clock(); \
} while (0)

#define TIMER_STOP(id)  do { \
    if (g_timing_enabled) { \
        g_timers[(id)].total_sec += \
            (double)(clock() - g_timers[(id)].start_tick) / CLOCKS_PER_SEC; \
        g_timers[(id)].call_count++; \
    } \
} while (0)

#define TIMER_REPORT()  do { \
    if (g_timing_enabled) timer_report(); \
} while (0)

/* ================================================================
 * 函数声明
 * ================================================================ */

void timer_init(void);
void timer_report(void);

#endif /* TIMER_H */
