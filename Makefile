# room-count — Makefile
# 多联骨牌房间计数项目

CC       = gcc
CFLAGS   = -std=c99 -O3 -march=native -Wall -Wextra -fopenmp
LDFLAGS  = -fopenmp

# 源文件
SRCDIR   = src
SRCS     = $(SRCDIR)/main.c $(SRCDIR)/enumerate.c $(SRCDIR)/hashset.c
OBJS     = $(SRCS:.c=.o)

# 输出
TARGET   = room-count.exe

# 默认目标
.PHONY: all
all: $(TARGET)

# 链接
$(TARGET): $(OBJS)
	$(CC) $(LDFLAGS) -o $@ $^

# 编译
$(SRCDIR)/%.o: $(SRCDIR)/%.c $(SRCDIR)/common.h
	$(CC) $(CFLAGS) -c -o $@ $<

# 显式依赖
$(SRCDIR)/main.o:       $(SRCDIR)/main.c       $(SRCDIR)/common.h $(SRCDIR)/enumerate.h
$(SRCDIR)/enumerate.o:  $(SRCDIR)/enumerate.c  $(SRCDIR)/common.h $(SRCDIR)/enumerate.h $(SRCDIR)/hashset.h
$(SRCDIR)/hashset.o:    $(SRCDIR)/hashset.c    $(SRCDIR)/common.h $(SRCDIR)/hashset.h

# 运行
.PHONY: run
run: $(TARGET)
	./$(TARGET)

# 清理
.PHONY: clean
clean:
	rm -f $(OBJS) $(TARGET)

# 完整重新构建
.PHONY: rebuild
rebuild: clean all

# 开发模式（调试符号 + 地址消毒器）
.PHONY: dev
dev: CFLAGS = -std=c99 -g -O0 -Wall -Wextra -fsanitize=address
dev: clean all

# 帮助
.PHONY: help
help:
	@echo "make          — 编译（-O3 优化）"
	@echo "make run      — 编译并运行"
	@echo "make clean    — 清理产物"
	@echo "make rebuild  — 清理后重新编译"
	@echo "make dev      — 调试版本（-g -O0 -fsanitize=address）"
