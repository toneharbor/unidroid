package baseline;

import com.github.unidbg.AndroidEmulator;
import com.github.unidbg.Emulator;
import com.github.unidbg.Module;
import com.github.unidbg.ModuleListener;
import com.github.unidbg.Symbol;
import com.github.unidbg.arm.HookStatus;
import com.github.unidbg.arm.backend.Unicorn2Factory;
import com.github.unidbg.linux.android.AndroidEmulatorBuilder;
import com.github.unidbg.hook.ReplaceCallback;
import com.github.unidbg.hook.xhook.IxHook;
import com.github.unidbg.linux.android.AndroidResolver;
import com.github.unidbg.linux.android.XHookImpl;
import com.github.unidbg.linux.android.dvm.AbstractJni;
import com.github.unidbg.linux.android.dvm.DalvikModule;
import com.github.unidbg.linux.android.dvm.DvmClass;
import com.github.unidbg.linux.android.dvm.DvmObject;
import com.github.unidbg.linux.android.dvm.StringObject;
import com.github.unidbg.linux.android.dvm.VM;
import com.github.unidbg.linux.android.dvm.array.ByteArray;
import com.github.unidbg.linux.android.dvm.jni.ProxyClassFactory;
import com.github.unidbg.linux.android.dvm.jni.ProxyDvmObject;
import com.github.unidbg.memory.Memory;
import com.github.unidbg.memory.MemoryBlock;
import com.github.unidbg.virtualmodule.android.AndroidModule;
import com.github.unidbg.virtualmodule.android.JniGraphics;

import java.io.File;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.HexFormat;
import java.util.List;
import java.util.Map;

/**
 * Runs unidbg's own samples with the Unicorn2 backend and prints every result as hex,
 * so minidbg can be checked against it byte for byte.
 * jna and tmessages are arm64-v8a; the rest are armeabi-v7a.
 *
 * Usage: Baseline <jna|tmessages|jna32|ttencrypt|tmessages32|signutil|qdreader|kanxue|thread>
 *        <dir with that sample's .so> [instances] [verbose]
 */
public class Baseline {

    public static void main(String[] args) throws Exception {
        String sample = args[0];
        File dir = new File(args[1]);
        int instances = args.length > 2 ? Integer.parseInt(args[2]) : 1;
        boolean verbose = args.length > 3 && Boolean.parseBoolean(args[3]);

        boolean is32 = !sample.equals("jna") && !sample.equals("tmessages");
        List<AndroidEmulator> emulators = new ArrayList<>();
        for (int i = 0; i < instances; i++) {
            AndroidEmulator emulator = (is32 ? AndroidEmulatorBuilder.for32Bit() : AndroidEmulatorBuilder.for64Bit())
                    .setProcessName("baseline")
                    .addBackendFactory(new Unicorn2Factory(false))
                    .build();
            if (sample.equals("thread")) {
                // Same as unidbg's ThreadTest: switch guest threads every 10000 instructions.
                emulator.getBackend().registerEmuCountHook(10000);
                emulator.getSyscallHandler().setEnableThreadDispatcher(true);
            }
            Memory memory = emulator.getMemory();
            memory.setLibraryResolver(new AndroidResolver(23));
            VM vm = emulator.createDalvikVM();
            vm.setDvmClassFactory(new ProxyClassFactory());
            vm.setVerbose(verbose);
            boolean print = i == instances - 1;
            switch (sample) {
                case "jna", "jna32" -> jna(emulator, vm, dir, print);
                case "tmessages", "tmessages32" -> tmessages(emulator, vm, dir, print);
                case "ttencrypt" -> ttencrypt(emulator, vm, dir, print);
                case "signutil" -> signutil(emulator, vm, dir, print);
                case "qdreader" -> qdreader(emulator, vm, dir, print);
                case "kanxue" -> kanxue(emulator, vm, dir, print);
                case "thread" -> thread(emulator, vm, dir, print);
                default -> throw new IllegalArgumentException("unknown sample: " + sample);
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

    private static void ttencrypt(AndroidEmulator emulator, VM vm, File dir, boolean print) {
        DalvikModule dm = vm.loadLibrary(new File(dir, "libttEncrypt.so"), false);
        dm.callJNI_OnLoad(emulator);
        DvmClass cUtils = vm.resolveClass("com/bytedance/frameworks/core/encrypt/TTEncryptUtils");
        byte[] data = new byte[16];
        ByteArray out = cUtils.callStaticJniMethodObject(emulator, "ttEncrypt([BI)[B", new ByteArray(vm, data), data.length);
        if (print) {
            System.out.println("ttEncrypt=" + HexFormat.of().formatHex(out.getValue()));
        }
    }

    // unidbg's com.anjuke.mobile.sign.SignUtil
    private static void signutil(AndroidEmulator emulator, VM vm, File dir, boolean print) {
        DalvikModule dm = vm.loadLibrary(new File(dir, "libsignutil.so"), false);
        DvmClass cSignUtil = vm.resolveClass("com/anjuke/mobile/sign/SignUtil");
        dm.callJNI_OnLoad(emulator);
        Map<String, byte[]> map = new HashMap<>();
        map.put("a", "b".getBytes(StandardCharsets.UTF_8));
        map.put("b", "b".getBytes(StandardCharsets.UTF_8));
        StringObject sign = cSignUtil.callStaticJniMethodObject(emulator,
                "getSign0(Ljava/lang/String;Ljava/lang/String;Ljava/util/Map;Ljava/lang/String;I)Ljava/lang/String;",
                "aa", "bb", ProxyDvmObject.createObject(vm, map), "cc", 10);
        if (print) {
            System.out.println("sign=" + sign.getValue());
        }
    }

    // unidbg's com.github.unidbg.android.QDReaderJni
    private static void qdreader(AndroidEmulator emulator, VM vm, File dir, boolean print) {
        // libcrypto.so of API 23 has no DES_set_key_unchecked; the test points it at DES_set_key.
        emulator.getMemory().addModuleListener(new ModuleListener() {
            @Override
            public void onLoaded(Emulator<?> e, Module module) {
                if ("libcrypto.so".equals(module.name)) {
                    Symbol setKey = module.findSymbolByName("DES_set_key", false);
                    if (module.findSymbolByName("DES_set_key_unchecked", false) == null && setKey != null) {
                        module.registerSymbol("DES_set_key_unchecked", setKey.getAddress());
                    }
                }
            }
        });
        DalvikModule dm = vm.loadLibrary(new File(dir, "libd-lib.so"), false);
        dm.callJNI_OnLoad(emulator);
        DvmClass d = vm.resolveClass("a/d");
        // The test makes free() in libd-lib.so a no-op.
        IxHook xHook = XHookImpl.getInstance(emulator);
        xHook.register("libd-lib.so", "free", new ReplaceCallback() {
            @Override
            public HookStatus onCall(Emulator<?> e, long originFunction) {
                return HookStatus.LR(e, 0);
            }
        });
        xHook.refresh();
        ByteArray out = d.callStaticJniMethodObject(emulator, "c(Ljava/lang/String;)[B",
                new StringObject(vm, "359250054370919||1551086094"));
        if (print) {
            System.out.println("c=" + HexFormat.of().formatHex(out.getValue()));
        }
    }

    // unidbg's com.kanxue.test2.MainActivity: try every three-letter string until jnitest says yes.
    private static void kanxue(AndroidEmulator emulator, VM vm, File dir, boolean print) {
        DalvikModule dm = vm.loadLibrary(new File(dir, "libnative-lib.so"), false);
        dm.callJNI_OnLoad(emulator);
        DvmObject<?> activity = vm.resolveClass("com/kanxue/test2/MainActivity").newObject(null);
        String letters = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
        int tries = 0;
        String found = null;
        search:
        for (char a : letters.toCharArray()) {
            for (char b : letters.toCharArray()) {
                for (char c : letters.toCharArray()) {
                    String candidate = "" + a + b + c;
                    tries++;
                    if (activity.callJniMethodBoolean(emulator, "jnitest(Ljava/lang/String;)Z", candidate)) {
                        found = candidate;
                        break search;
                    }
                }
            }
        }
        if (print) {
            System.out.println("found=" + found);
            System.out.println("tries=" + tries);
        }
    }

    // unidbg's com.github.unidbg.android.ThreadTest
    private static void thread(AndroidEmulator emulator, VM vm, File dir, boolean print) {
        vm.setJni(new AbstractJni() {
        });
        DalvikModule dm = vm.loadLibrary(new File(dir, "libthread-lib.so"), false);
        dm.callJNI_OnLoad(emulator);
        DvmObject<?> ret = vm.resolveClass("com/mpt/jnithread/MainActivity")
                .callStaticJniMethodObject(emulator, "stringFromJNI()Ljava/lang/String;");
        if (print) {
            System.out.println("stringFromJNI=" + ret.getValue());
        }
    }
}
