using System;
using System.ComponentModel;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;

// 所有结构均按 winnt.h / processthreadsapi.h 的原生字段顺序布局。
public static class BoundedProcess
{
    const uint CREATE_SUSPENDED = 0x00000004;
    const uint CREATE_NO_WINDOW = 0x08000000;
    const uint STARTF_USESTDHANDLES = 0x00000100;
    const uint JOB_OBJECT_LIMIT_PROCESS_MEMORY = 0x00000100;
    const uint JOB_OBJECT_LIMIT_JOB_MEMORY = 0x00000200;
    const uint JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE = 0x00002000;
    const uint WAIT_OBJECT_0 = 0;
    const uint WAIT_TIMEOUT = 258;
    const uint FILE_GENERIC_WRITE = 0x40000000;
    const uint FILE_GENERIC_READ = 0x80000000;
    const uint FILE_SHARE_READ = 1;
    const uint CREATE_ALWAYS = 2;
    const uint OPEN_EXISTING = 3;
    const uint FILE_ATTRIBUTE_NORMAL = 0x80;
    static readonly IntPtr INVALID_HANDLE_VALUE = new IntPtr(-1);

    [StructLayout(LayoutKind.Sequential)]
    struct BasicLimitInfo
    {
        public long PerProcessUserTimeLimit, PerJobUserTimeLimit;
        public uint LimitFlags;
        public UIntPtr MinimumWorkingSetSize, MaximumWorkingSetSize;
        public uint ActiveProcessLimit;
        public UIntPtr Affinity;
        public uint PriorityClass, SchedulingClass;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct IoCounters
    {
        public ulong ReadOperationCount, WriteOperationCount, OtherOperationCount;
        public ulong ReadTransferCount, WriteTransferCount, OtherTransferCount;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct ExtendedLimitInfo
    {
        public BasicLimitInfo BasicLimitInformation;
        public IoCounters IoInfo;
        public UIntPtr ProcessMemoryLimit, JobMemoryLimit;
        public UIntPtr PeakProcessMemoryUsed, PeakJobMemoryUsed;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct CompletionPortInfo
    {
        public IntPtr CompletionKey, CompletionPort;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct SecurityAttributes
    {
        public uint Length;
        public IntPtr SecurityDescriptor;
        [MarshalAs(UnmanagedType.Bool)] public bool InheritHandle;
    }
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct StartupInfo
    {
        public uint Size;
        public string Reserved, Desktop, Title;
        public uint X, Y, XSize, YSize, XCountChars, YCountChars, FillAttribute, Flags;
        public ushort ShowWindow, Reserved2Size;
        public IntPtr Reserved2, StdInput, StdOutput, StdError;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct ProcessInfo
    {
        public IntPtr Process, Thread;
        public uint ProcessId, ThreadId;
    }

    [DllImport("kernel32.dll", SetLastError = true)] static extern IntPtr CreateJobObject(IntPtr attributes, string name);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool SetInformationJobObject(IntPtr job, int infoClass, ref ExtendedLimitInfo info, uint length);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool SetInformationJobObject(IntPtr job, int infoClass, ref CompletionPortInfo info, uint length);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool QueryInformationJobObject(IntPtr job, int infoClass, out ExtendedLimitInfo info, uint length, IntPtr returned);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool TerminateJobObject(IntPtr job, uint exitCode);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool TerminateProcess(IntPtr process, uint exitCode);
    [DllImport("kernel32.dll", SetLastError = true)] static extern IntPtr CreateIoCompletionPort(IntPtr file, IntPtr existing, UIntPtr key, uint threads);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool GetQueuedCompletionStatus(IntPtr port, out uint bytes, out UIntPtr key, out IntPtr overlapped, uint milliseconds);
    [DllImport("kernel32.dll", SetLastError = true)] static extern uint ResumeThread(IntPtr thread);
    [DllImport("kernel32.dll", SetLastError = true)] static extern uint WaitForSingleObject(IntPtr handle, uint milliseconds);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool GetExitCodeProcess(IntPtr process, out uint code);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool CloseHandle(IntPtr handle);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool CreateProcess(string application, StringBuilder commandLine, IntPtr processAttributes,
        IntPtr threadAttributes, bool inheritHandles, uint flags, IntPtr environment, string directory,
        ref StartupInfo startup, out ProcessInfo process);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr CreateFile(string path, uint access, uint share, ref SecurityAttributes attributes,
        uint disposition, uint flags, IntPtr template);

    public sealed class Result
    {
        public int ExitCode { get; set; }
        public bool TimedOut { get; set; }
        public bool MemoryLimitEvent { get; set; }
        public ulong PeakJobMemoryBytes { get; set; }
        public ulong PeakProcessMemoryBytes { get; set; }
        public double ElapsedSeconds { get; set; }
    }

    static Exception Error(string operation) { return new Win32Exception(Marshal.GetLastWin32Error(), operation); }
    static void Check(bool ok, string operation) { if (!ok) throw Error(operation); }
    static void Close(ref IntPtr handle)
    {
        if (handle != IntPtr.Zero && handle != INVALID_HANDLE_VALUE) CloseHandle(handle);
        handle = IntPtr.Zero;
    }
    static string Quote(string value)
    {
        // Windows 命令行反斜杠/引号规则；不经过 cmd.exe 或 PowerShell 解析参数。
        var output = new StringBuilder("\"");
        int slashes = 0;
        foreach (char c in value)
        {
            if (c == '\\') { slashes++; continue; }
            if (c == '"') output.Append('\\', slashes * 2 + 1);
            else output.Append('\\', slashes);
            slashes = 0;
            output.Append(c);
        }
        output.Append('\\', slashes * 2).Append('"');
        return output.ToString();
    }

    public static Result Run(string executable, string[] arguments, string stdoutPath, string stderrPath,
        ulong memoryBytes, uint timeoutMilliseconds)
    {
        if (IntPtr.Size != 8) throw new PlatformNotSupportedException("需要 64 位 Windows PowerShell");
        if (Marshal.SizeOf(typeof(BasicLimitInfo)) != 64 || Marshal.SizeOf(typeof(ExtendedLimitInfo)) != 144 ||
            Marshal.SizeOf(typeof(StartupInfo)) != 104 || Marshal.SizeOf(typeof(ProcessInfo)) != 24)
            throw new PlatformNotSupportedException("Win32 结构布局与 64 位 Windows 不匹配");
        if (memoryBytes == 0 || timeoutMilliseconds == 0) throw new ArgumentOutOfRangeException("限制必须大于零");
        IntPtr job = IntPtr.Zero, port = IntPtr.Zero, stdin = IntPtr.Zero, stdout = IntPtr.Zero, stderr = IntPtr.Zero;
        ProcessInfo process = new ProcessInfo();
        bool created = false, assigned = false;
        var watch = new Stopwatch();
        try
        {
            job = CreateJobObject(IntPtr.Zero, null);
            if (job == IntPtr.Zero) throw Error("CreateJobObject");
            var limit = new ExtendedLimitInfo();
            limit.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_PROCESS_MEMORY |
                JOB_OBJECT_LIMIT_JOB_MEMORY | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            limit.ProcessMemoryLimit = new UIntPtr(memoryBytes);
            limit.JobMemoryLimit = new UIntPtr(memoryBytes);
            Check(SetInformationJobObject(job, 9, ref limit, (uint)Marshal.SizeOf(typeof(ExtendedLimitInfo))), "SetInformationJobObject limits");
            port = CreateIoCompletionPort(INVALID_HANDLE_VALUE, IntPtr.Zero, UIntPtr.Zero, 1);
            if (port == IntPtr.Zero) throw Error("CreateIoCompletionPort");
            var association = new CompletionPortInfo { CompletionKey = new IntPtr(1), CompletionPort = port };
            Check(SetInformationJobObject(job, 7, ref association, (uint)Marshal.SizeOf(typeof(CompletionPortInfo))), "SetInformationJobObject port");

            var attributes = new SecurityAttributes { Length = (uint)Marshal.SizeOf(typeof(SecurityAttributes)), InheritHandle = true };
            stdin = CreateFile("NUL", FILE_GENERIC_READ, FILE_SHARE_READ, ref attributes, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, IntPtr.Zero);
            stdout = CreateFile(stdoutPath, FILE_GENERIC_WRITE, FILE_SHARE_READ, ref attributes, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, IntPtr.Zero);
            stderr = CreateFile(stderrPath, FILE_GENERIC_WRITE, FILE_SHARE_READ, ref attributes, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, IntPtr.Zero);
            if (stdin == INVALID_HANDLE_VALUE || stdout == INVALID_HANDLE_VALUE || stderr == INVALID_HANDLE_VALUE) throw Error("CreateFile stdio");
            var startup = new StartupInfo { Size = (uint)Marshal.SizeOf(typeof(StartupInfo)), Flags = STARTF_USESTDHANDLES,
                StdInput = stdin, StdOutput = stdout, StdError = stderr };
            var command = new StringBuilder(Quote(executable));
            foreach (string argument in arguments) command.Append(' ').Append(Quote(argument));
            watch.Start();
            Check(CreateProcess(executable, command, IntPtr.Zero, IntPtr.Zero, true,
                CREATE_SUSPENDED | CREATE_NO_WINDOW, IntPtr.Zero, System.IO.Path.GetDirectoryName(executable), ref startup, out process), "CreateProcess");
            created = true;
            Check(AssignProcessToJobObject(job, process.Process), "AssignProcessToJobObject");
            assigned = true;
            if (ResumeThread(process.Thread) == UInt32.MaxValue) throw Error("ResumeThread");
            uint wait = WaitForSingleObject(process.Process, timeoutMilliseconds);
            var result = new Result { TimedOut = wait == WAIT_TIMEOUT };
            if (wait != WAIT_OBJECT_0 && wait != WAIT_TIMEOUT) throw Error("WaitForSingleObject");
            if (result.TimedOut)
            {
                Check(TerminateJobObject(job, 124), "TerminateJobObject timeout");
                Check(WaitForSingleObject(process.Process, 5000) == WAIT_OBJECT_0, "WaitForSingleObject after timeout");
            }
            watch.Stop();
            result.ElapsedSeconds = watch.Elapsed.TotalSeconds;
            uint exitCode;
            Check(GetExitCodeProcess(process.Process, out exitCode), "GetExitCodeProcess");
            result.ExitCode = unchecked((int)exitCode);
            ExtendedLimitInfo observed;
            Check(QueryInformationJobObject(job, 9, out observed, (uint)Marshal.SizeOf(typeof(ExtendedLimitInfo)), IntPtr.Zero), "QueryInformationJobObject");
            result.PeakJobMemoryBytes = observed.PeakJobMemoryUsed.ToUInt64();
            result.PeakProcessMemoryBytes = observed.PeakProcessMemoryUsed.ToUInt64();
            uint message; UIntPtr key; IntPtr overlapped;
            while (GetQueuedCompletionStatus(port, out message, out key, out overlapped, 0))
                if (message == 9 || message == 10) result.MemoryLimitEvent = true;
            return result;
        }
        finally
        {
            // 子进程若仍在运行，关闭 job 会终止整棵进程树。
            if (created && !assigned && process.Process != IntPtr.Zero) TerminateProcess(process.Process, 125);
            Close(ref process.Thread); Close(ref process.Process);
            Close(ref stdin); Close(ref stdout); Close(ref stderr);
            Close(ref job); Close(ref port);
        }
    }
}
