package baseline;

import com.github.unidbg.AndroidEmulator;
import com.github.unidbg.Module;
import com.github.unidbg.Symbol;
import com.github.unidbg.arm.backend.Unicorn2Factory;
import com.github.unidbg.linux.android.AndroidEmulatorBuilder;
import com.github.unidbg.linux.android.AndroidResolver;
import com.github.unidbg.linux.android.dvm.DalvikModule;
import com.github.unidbg.linux.android.dvm.DvmClass;
import com.github.unidbg.linux.android.dvm.DvmObject;
import com.github.unidbg.linux.android.dvm.VM;
import com.github.unidbg.linux.android.dvm.array.ByteArray;
import com.github.unidbg.linux.android.dvm.jni.ProxyClassFactory;
import com.github.unidbg.memory.Memory;
import com.github.unidbg.memory.MemoryBlock;
import com.github.unidbg.virtualmodule.android.AndroidModule;
import com.github.unidbg.virtualmodule.android.JniGraphics;

import java.io.File;
import java.util.ArrayList;
import java.util.HexFormat;
import java.util.List;

/**
 * Runs unidbg's own arm64 samples with the Unicorn2 backend and prints every result as hex,
 * so minidbg can be checked against it byte for byte.
 *
 * Usage: Baseline <jna|tmessages> <dir with the arm64-v8a samples> [instances] [verbose]
 */
public class Baseline {

    public static void main(String[] args) throws Exception {
        String sample = args[0];
        File dir = new File(args[1]);
        int instances = args.length > 2 ? Integer.parseInt(args[2]) : 1;
        boolean verbose = args.length > 3 && Boolean.parseBoolean(args[3]);

        List<AndroidEmulator> emulators = new ArrayList<>();
        for (int i = 0; i < instances; i++) {
            AndroidEmulator emulator = AndroidEmulatorBuilder.for64Bit()
                    .setProcessName("baseline")
                    .addBackendFactory(new Unicorn2Factory(false))
                    .build();
            Memory memory = emulator.getMemory();
            memory.setLibraryResolver(new AndroidResolver(23));
            VM vm = emulator.createDalvikVM();
            vm.setDvmClassFactory(new ProxyClassFactory());
            vm.setVerbose(verbose);
            boolean print = i == instances - 1;
            if (sample.equals("jna")) {
                jna(emulator, vm, dir, print);
            } else {
                tmessages(emulator, vm, dir, print);
            }
            emulators.add(emulator);
        }
        System.out.println("instances=" + emulators.size());
        for (AndroidEmulator emulator : emulators) {
            emulator.close();
        }
    }

    private static void jna(AndroidEmulator emulator, VM vm, File dir, boolean print) {
        DalvikModule dm = vm.loadLibrary(new File(dir, "libjnidispatch.so"), false);
        dm.callJNI_OnLoad(emulator);
        Module module = dm.getModule();
        DvmClass cNative = vm.resolveClass("com/sun/jna/Native");

        Symbol propertyGet = module.findSymbolByName("__system_property_get", true);
        MemoryBlock block = emulator.getMemory().malloc(0x10, false);
        Number len = propertyGet.call(emulator, "ro.build.version.sdk", block.getPointer());
        String sdk = new String(block.getPointer().getByteArray(0, len.intValue()));
        block.free();

        Number ptr = cNative.callStaticJniMethodLong(emulator, "malloc(J)J", 0x20);
        DvmObject<?> version = cNative.callStaticJniMethodObject(emulator, "getNativeVersion()Ljava/lang/String;");
        DvmObject<?> checksum = cNative.callStaticJniMethodObject(emulator, "getAPIChecksum()Ljava/lang/String;");
        int size = cNative.callStaticJniMethodInt(emulator, "sizeof(I)I", 0);
        if (print) {
            System.out.println("sdk=" + sdk);
            System.out.println("malloc_nonzero=" + (ptr.longValue() != 0));
            System.out.println("getNativeVersion=" + version.getValue());
            System.out.println("getAPIChecksum=" + checksum.getValue());
            System.out.println("sizeof(0)=" + size);
        }
    }

    private static void tmessages(AndroidEmulator emulator, VM vm, File dir, boolean print) {
        Memory memory = emulator.getMemory();
        new JniGraphics(emulator, vm).register(memory);
        new AndroidModule(emulator, vm).register(memory);
        DalvikModule dm = vm.loadLibrary(new File(dir, "libtmessages.29.so"), true);
        dm.callJNI_OnLoad(emulator);
        DvmClass cUtilities = vm.resolveClass("org/telegram/messenger/Utilities");

        ByteArray cbc = new ByteArray(vm, new byte[16]);
        cUtilities.callStaticJniMethod(emulator, "aesCbcEncryptionByteArray([B[B[BIIII)V",
                cbc, new byte[32], new byte[16], 0, 16, 0, 0);

        ByteArray ctr = new ByteArray(vm, new byte[16]);
        cUtilities.callStaticJniMethod(emulator, "aesCtrDecryptionByteArray([B[B[BIII)V",
                ctr, new byte[32], new byte[16], 0, 16, 0);

        ByteArray dst = new ByteArray(vm, new byte[64]);
        long start = System.currentTimeMillis();
        cUtilities.callStaticJniMethod(emulator, "pbkdf2([B[B[BI)V",
                "123456".getBytes(), new byte[8], dst, 100000);
        long pbkdf2Ms = System.currentTimeMillis() - start;

        if (print) {
            HexFormat hex = HexFormat.of();
            System.out.println("aesCbc=" + hex.formatHex(cbc.getValue()));
            System.out.println("aesCtr=" + hex.formatHex(ctr.getValue()));
            System.out.println("pbkdf2=" + hex.formatHex(dst.getValue()));
            System.out.println("pbkdf2_ms=" + pbkdf2Ms);
        }
    }
}
