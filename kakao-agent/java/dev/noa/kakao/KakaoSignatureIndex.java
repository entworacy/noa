package dev.noa.kakao;

import java.util.ArrayList;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/** Decodes the compact signature index produced once by the injected Rust runtime. */
final class KakaoSignatureIndex {
    private KakaoSignatureIndex() {}

    static Data decode(String encoded) {
        if (encoded == null || encoded.isEmpty()) {
            throw new IllegalArgumentException("Rust DEX signature index is empty");
        }
        Map<String, List<String>> classesBySource = new LinkedHashMap<>();
        Map<String, List<MethodRef>> operations = new LinkedHashMap<>();
        for (String row : encoded.split("\n", -1)) {
            if (row.isEmpty()) {
                continue;
            }
            String[] columns = row.split("\t", -1);
            if (columns.length < 3) {
                throw new IllegalArgumentException("malformed Rust DEX index row");
            }
            if ("S".equals(columns[0])) {
                List<String> classes = classesBySource.computeIfAbsent(
                        columns[1], ignored -> new ArrayList<>());
                for (int index = 2; index < columns.length; index++) {
                    if (columns[index].isEmpty()) {
                        throw new IllegalArgumentException("empty DEX class name");
                    }
                    classes.add(columns[index]);
                }
            } else if ("O".equals(columns[0])) {
                if ((columns.length - 2) % 2 != 0) {
                    throw new IllegalArgumentException("malformed DEX operation row");
                }
                List<MethodRef> methods = operations.computeIfAbsent(
                        columns[1], ignored -> new ArrayList<>());
                for (int index = 2; index < columns.length; index += 2) {
                    if (columns[index].isEmpty() || columns[index + 1].isEmpty()) {
                        throw new IllegalArgumentException("empty DEX method reference");
                    }
                    methods.add(new MethodRef(columns[index], columns[index + 1]));
                }
            } else {
                throw new IllegalArgumentException("unknown Rust DEX index row type");
            }
        }
        if (classesBySource.isEmpty()) {
            throw new IllegalArgumentException("Rust DEX signature index has no source classes");
        }
        return new Data(immutable(classesBySource), immutable(operations));
    }

    private static <T> Map<String, List<T>> immutable(Map<String, List<T>> source) {
        Map<String, List<T>> result = new LinkedHashMap<>();
        for (Map.Entry<String, List<T>> entry : source.entrySet()) {
            result.put(entry.getKey(), Collections.unmodifiableList(entry.getValue()));
        }
        return Collections.unmodifiableMap(result);
    }

    static final class Data {
        final Map<String, List<String>> classesBySource;
        final Map<String, List<MethodRef>> operations;

        Data(
                Map<String, List<String>> classesBySource,
                Map<String, List<MethodRef>> operations) {
            this.classesBySource = classesBySource;
            this.operations = operations;
        }
    }

    static final class MethodRef {
        final String owner;
        final String name;

        MethodRef(String owner, String name) {
            this.owner = owner;
            this.name = name;
        }

        @Override
        public String toString() {
            return owner + "." + name;
        }
    }
}
