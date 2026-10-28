package org.nanobug.maven;
import java.util.Map;

/** Bounded model serialization needs only JSON scalars, lists and insertion-ordered string maps. */
final class Json {
    /** This internal writer handles already admitted model data and never interprets executable text. */
    static String write(Object value) {
        if (value == null) return "null";
        if (value instanceof Number || value instanceof Boolean) return value.toString();
        if (value instanceof Map) {
            StringBuilder text = new StringBuilder("{");
            for (Object item : ((Map<?,?>) value).entrySet()) {
                Map.Entry<?,?> entry = (Map.Entry<?,?>) item;
                if (text.length() > 1) text.append(',');
                text.append(write(entry.getKey().toString())).append(':').append(write(entry.getValue()));
            }
            return text.append('}').toString();
        }
        if (value instanceof Iterable) {
            StringBuilder text = new StringBuilder("[");
            for (Object item : (Iterable<?>) value) {
                if (text.length() > 1) text.append(',');
                text.append(write(item));
            }
            return text.append(']').toString();
        }
        StringBuilder text = new StringBuilder("\"");
        for (char ch : value.toString().toCharArray()) {
            if (ch == '"' || ch == '\\') text.append('\\').append(ch);
            else if (ch < 32) text.append(String.format("\\u%04x", (int) ch));
            else text.append(ch);
        }
        return text.append('"').toString();
    }
}
