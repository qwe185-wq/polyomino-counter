/**
 * timer.h — 增强计时 Profiler
 *
 * 用法: ./room-count.exe 5 --time
 *
 * 新增:
 *   - 壁钟时间 (omp_get_wtime) — 并行模式精确
 *   - TIMER_WAIT_LOCK — 临界区等锁时间
 *   - 每代耗时输出
 */

#ifndef TIMER_H
#define TIMER_H

#include <time.h>
#include <stdio.h>

#ifdef _OPENMP
#include <omp.h>
#define TIMER_NOW() omp_get_wtime()
#else
#define TIMER_NOW() ((double)clock() / CLOCKS_PER_SEC)
#endif

enum {
    TIMER_TOTAL = 0,       /* 程序总耗时 */
    TIMER_EXTRACT,         /* 掩码 → 坐标提取 */
    TIMER_FRONTIER,        /* 前沿计算 */
    TIMER_GROW,            /* 扩展格子 + 构造新掩码 */
    TIMER_ORIENT_LOOKUP,   /* 方向集查找 */
    TIMER_CANONICAL,       /* One-sided 规范化（计算 4 方向） */
    TIMER_HOLE,            /* Flood fill 洞检测 */
    TIMER_HS_INSERT,       /* 方向集插入（临界区内） */
    TIMER_WAIT_LOCK,       /* 等锁时间 */
    TIMER_MERGE,           /* 代末合并 */
    TIMER_COUNT
};

typedef struct {
    const char *name;
    double total_sec;
    double start;
    int call_count;
} TimerSlot;

extern TimerSlot g_timers[TIMER_COUNT];
extern int g_timing_enabled;

/* 线程安全的 stop — 用 omp atomic 累加 */
#define TIMER_START(id)  do { \
    if (g_timing_enabled) g_timers[(id)].start = TIMER_NOW(); \
} while (0)

#define TIMER_STOP(id)  do { \
    if (g_timing_enabled) { \
        double dt = TIMER_NOW() - g_timers[(id)].start; \
        _timer_add((id), dt); \
    } \
} while (0)

/* 等锁专用: 记录开始等锁时间，返回0供后续 STOP_WAIT 使用 */
#define TIMER_START_WAIT()  (g_timing_enabled ? TIMER_NOW() : 0.0)
/* 计算等锁耗时并累加 */
#define TIMER_STOP_WAIT(t0)  do { \
    if (g_timing_enabled) { \
        double dt = TIMER_NOW() - (t0); \
        _timer_add(TIMER_WAIT_LOCK, dt); \
    } \
} while (0)

#define TIMER_INIT()   timer_init()
#define TIMER_REPORT() do { if (g_timing_enabled) timer_report(); } while (0)

/* 内部: 线程安全累加 */
void _timer_add(int id, double dt);

/* 每代计时辅助 */
double timer_gen_start(void);   /* 返回起始时间戳 */
void   timer_gen_print(int size, double t0, int wait_ms,
                       int nshapes, int gen);

void timer_init(void);
void timer_report(void);

#endif
