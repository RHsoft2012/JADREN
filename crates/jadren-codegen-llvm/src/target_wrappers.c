/*
 * LLVM's llvm-c/Target.h defines these helpers as static inline functions.
 * llvm-sys normally compiles an equivalent wrapper through llvm-config, which
 * the official Windows binary distribution does not ship. Jadren 0.1 targets
 * x86-64 first, so these wrappers deliberately initialize only the X86 family.
 */

typedef int LLVMBool;

extern void LLVMInitializeX86TargetInfo(void);
extern void LLVMInitializeX86Target(void);
extern void LLVMInitializeX86TargetMC(void);
extern void LLVMInitializeX86AsmPrinter(void);
extern void LLVMInitializeX86AsmParser(void);
extern void LLVMInitializeX86Disassembler(void);

void LLVM_InitializeAllTargetInfos(void) { LLVMInitializeX86TargetInfo(); }
void LLVM_InitializeAllTargets(void) { LLVMInitializeX86Target(); }
void LLVM_InitializeAllTargetMCs(void) { LLVMInitializeX86TargetMC(); }
void LLVM_InitializeAllAsmPrinters(void) { LLVMInitializeX86AsmPrinter(); }
void LLVM_InitializeAllAsmParsers(void) { LLVMInitializeX86AsmParser(); }
void LLVM_InitializeAllDisassemblers(void) { LLVMInitializeX86Disassembler(); }

LLVMBool LLVM_InitializeNativeTarget(void) {
    LLVMInitializeX86TargetInfo();
    LLVMInitializeX86Target();
    LLVMInitializeX86TargetMC();
    return 0;
}

LLVMBool LLVM_InitializeNativeAsmParser(void) {
    LLVMInitializeX86AsmParser();
    return 0;
}

LLVMBool LLVM_InitializeNativeAsmPrinter(void) {
    LLVMInitializeX86AsmPrinter();
    return 0;
}

LLVMBool LLVM_InitializeNativeDisassembler(void) {
    LLVMInitializeX86Disassembler();
    return 0;
}
