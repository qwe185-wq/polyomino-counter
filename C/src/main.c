/**
 * main.c — 入口程序
 *
 * 运行多联骨牌房间枚举，输出 n=1..max_n 的结果表。
 *
 * 用法:
 *   room-count.exe [n] [--time]
 *     n      — 网格尺寸（1..6，默认 6）
 *     --time — 开启函数级计时 profiler
 */

#include "enumerate.h"
#include "timer.h"
#include <time.h>
#include <string.h>

/* ================================================================
 * 全局计时器
 * ================================================================ */

TimerSlot g_timers[TIMER_COUNT];
int g_timing_enabled = 0;

/* 线程安全累加 */
void _timer_add(int id, double dt) {
#ifdef _OPENMP
    #pragma omp atomic
#endif
    g_timers[id].total_sec += dt;
#ifdef _OPENMP
    #pragma omp atomic
#endif
    g_timers[id].call_count++;
}

void timer_init(void) {
    g_timers[TIMER_TOTAL].name       = "总耗时";
    g_timers[TIMER_EXTRACT].name     = "提取坐标+包围盒";
    g_timers[TIMER_FRONTIER].name    = "前沿计算";
    g_timers[TIMER_GROW].name        = "扩展+构造掩码";
    g_timers[TIMER_ORIENT_LOOKUP].name = "方向集查找";
    g_timers[TIMER_CANONICAL].name   = "规范化(4方向)";
    g_timers[TIMER_HOLE].name        = "Flood fill 洞检测";
    g_timers[TIMER_HS_INSERT].name   = "方向集插入";
    g_timers[TIMER_WAIT_LOCK].name   = "等锁(临界区排队)";
    g_timers[TIMER_MERGE].name       = "代末合并";
}

double timer_gen_start(void) {
    return TIMER_NOW();
}

void timer_gen_print(int size, double t0, int wait_ms,
                     int nshapes, int gen) {
    double elapsed = TIMER_NOW() - t0;
    double rate = (elapsed > 0) ? (nshapes / elapsed / 1000.0) : 0;
    fprintf(stderr,
            "  [计时] 格=%2d 耗时=%6.1fs  等锁=%5.1fs  "
            "形状=%d 新=%d  速率=%.0fK/s\n",
            size, elapsed, wait_ms / 1000.0, nshapes, gen, rate);
}

void timer_report(void) {
    double total = g_timers[TIMER_TOTAL].total_sec;
    if (total <= 0.0) total = 0.001;  /* 防止除零 */

    printf("\n");
    printf("  ╔══════════════════════════════════════════════════════════════╗\n");
    printf("  ║           ⏱  函数级计时 Profiler 报告                        ║\n");
    printf("  ╠══════════════════════════╤══════════╤══════════╤════════════╣\n");
    printf("  ║ 函数                     │   调用次数  │  耗时(秒) │  占比       ║\n");
    printf("  ╟──────────────────────────┼──────────┼──────────┼────────────╢\n");

    for (int i = 0; i < TIMER_COUNT; i++) {
        double pct = (g_timers[i].total_sec / total) * 100.0;
        printf("  ║ %-24s │ %8d │ %8.3f │ %6.1f%%    ║\n",
               g_timers[i].name,
               g_timers[i].call_count,
               g_timers[i].total_sec,
               pct);
    }

    printf("  ╚══════════════════════════╧══════════╧══════════╧════════════╝\n");

    /* 简易柱状图 */
    printf("\n  ── 耗时占比可视化 ──\n");
    for (int i = 0; i < TIMER_COUNT; i++) {
        double pct = (g_timers[i].total_sec / total) * 100.0;
        int bar_len = (int)(pct / 2.0 + 0.5);  /* 每 2% 一个字符 */
        if (bar_len > 60) bar_len = 60;
        if (bar_len < 1 && pct > 0.0) bar_len = 1;

        printf("  %-24s │", g_timers[i].name);
        for (int j = 0; j < bar_len; j++) printf("█");
        printf(" %.1f%%\n", pct);
    }
    printf("\n");
}

/* ================================================================
 * 主函数
 * ================================================================ */

int main(int argc, char **argv) {
    int max_n = MAX_N;

    /* 解析命令行参数 */
    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "--time") == 0) {
            g_timing_enabled = 1;
        } else {
            int val = atoi(argv[i]);
            if (val >= 1 && val <= MAX_N) {
                max_n = val;
            }
        }
    }

    /* 初始化计时器 */
    TIMER_INIT();

    printf("═══════════════════════════════════════════════════════════\n");
    printf("  多联骨牌房间计数 — One-sided Polyomino 枚举\n");
    printf("  n×n 正方形网格，每个格子是一个房间单元\n");
    printf("  房间 = 墙+门围成的连通区域（≤ n×n）\n");
    printf("  去重规则：允许旋转+平移，禁止翻转（One-sided）\n");
    printf("  目标 n = %d", max_n);
    if (g_timing_enabled) printf("  [⏱ Profiler 已开启]");
    printf("\n");
    printf("═══════════════════════════════════════════════════════════\n\n");

    /* ---------- [计时] 总耗时 ---------- */
    TIMER_START(TIMER_TOTAL);

    clock_t start = clock();

    int count;
    RoomCount *results = enumerate_all(max_n, &count);

    clock_t elapsed = clock() - start;
    double seconds = (double)elapsed / CLOCKS_PER_SEC;

    TIMER_STOP(TIMER_TOTAL);

    if (!results) {
        fprintf(stderr, "错误：枚举失败（内存不足？）\n");
        return 1;
    }

    printf("\n");
    printf("  ╔══════════════════════════════════════════════════════╗\n");
    printf("  ║              枚举结果（One-sided）                    ║\n");
    printf("  ╠═════╤══════════╤══════════╤══════════╤═══════════════╣\n");
    printf("  ║  n  │  总房间数  │  无洞(亏格0) │  有洞(亏格≥1) ║\n");
    printf("  ╟─────┼──────────┼──────────┼──────────┼───────────────╢\n");

    for (int i = 0; i < count; i++) {
        printf("  ║ %-3d │ %8d │ %8d │ %8d ║\n",
               results[i].n, results[i].total,
               results[i].no_hole, results[i].has_hole);
    }

    printf("  ╚═════╧══════════╧══════════╧══════════╧═══════════════╝\n");

    printf("\n  ⏱ 运行耗时: %.2f 秒 (wall clock)\n", seconds);
    printf("  📊 唯一形状总数: %d (≤ %d×%d 包围矩形)\n",
           results[count - 1].total, max_n, max_n);

    /* 输出便于复制的结果摘要 */
    printf("\n  ── 结果摘要（n, total, no_hole, has_hole）──\n");
    for (int i = 0; i < count; i++) {
        printf("  n=%d: total=%d, no_hole=%d, has_hole=%d\n",
               results[i].n, results[i].total,
               results[i].no_hole, results[i].has_hole);
    }

    free(results);

    /* ---------- [计时] 输出报告 ---------- */
    TIMER_REPORT();

    return 0;
}
