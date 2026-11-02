package dev.noa.kakao;
import java.util.*;
/** Fixture-only DEX source index; production resolvers are compiled unchanged. */
public final class KakaoSignatureResolver {
    static final Map<String, List<Class<?>>> sources = new HashMap<>();
    static List<Class<?>> sourceClasses(String source) throws ClassNotFoundException {
        List<Class<?>> classes = sources.get(source);
        if (classes == null) throw new ClassNotFoundException(source);
        return classes;
    }
    public static String[] sourceClassNames(String source) throws Exception {
        return sourceClasses(source).stream().map(Class::getName).toArray(String[]::new);
    }
    static ClassLoader appLoader() { return KakaoSignatureResolver.class.getClassLoader(); }
}
