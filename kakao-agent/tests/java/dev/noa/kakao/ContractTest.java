package dev.noa.kakao;
import java.util.*;

public final class ContractTest {
    enum Method { MSG, KICKMEM }
    static class Header {
        final int a, d;
        final short b;
        final Method c;
        Header(int id, short status, Method method, int length) { a=id; b=status; c=method; d=length; }
    }
    static final class RenamedHeader extends Header {
        // Real APK headers declare their fields; this subclass must be rejected.
        RenamedHeader(int a, short b, Method c, int d) { super(a,b,c,d); }
    }
    static class NewHeader {
        final int a, d;
        final short b;
        final Method c;
        NewHeader(int id, short status, Method method, int length) { a=id; b=status; c=method; d=length; }
    }
    static class Bson { public String toString() { return "{\"chatId\":123}"; } }
    static class Body { final Bson a; Body(Bson value) { a=value; } }
    static class NewBody { final Bson a; NewBody(Bson value) { a=value; } }
    static class PacketBase { final Header header; final Body body; PacketBase() { header=new Header(12,(short)-3,Method.KICKMEM,23); body=new Body(new Bson()); } }
    static class Request extends PacketBase {}
    static class Response extends PacketBase {}
    static class NewPacketBase { final NewHeader h=new NewHeader(12,(short)-3,Method.KICKMEM,23); final NewBody b=new NewBody(new Bson()); }
    static class NewRequest extends NewPacketBase {}
    static class NewResponse extends NewPacketBase {}
    static class AmbiguousPacket extends NewPacketBase { final NewHeader duplicate=null; }
    static class UnknownPacket { final String body="bad"; }
    static class LegacyManager {
        static final Companion companion=new Companion();
        static class Companion {
            final LegacyManager value=new LegacyManager();
            public LegacyManager get() { return value; }
        }
    }
    static class CurrentManager {}
    static class Provider {
        static final CurrentManager instance=new CurrentManager();
        public static CurrentManager a() { return instance; }
    }
    static class SecondProvider { public static CurrentManager x() { return Provider.instance; } }
    static class NullProvider { public static CurrentManager a() { return null; } }
    interface Throwing { void run() throws Exception; }
    static void rejects(Throwing operation) throws Exception {
        try { operation.run(); } catch (ReflectiveOperationException | IllegalStateException expected) { return; }
        throw new AssertionError("invalid contract was accepted");
    }
    static void packet(Object packet) throws Exception {
        LocoPacketReader.verify(packet.getClass());
        Object[] values=LocoPacketReader.read(packet);
        if (!Arrays.equals(Arrays.copyOf(values,4),new Object[]{12,(short)-3,Method.KICKMEM,23})) throw new AssertionError(Arrays.toString(values));
        if (!"{\"chatId\":123}".equals(values[4].toString())) throw new AssertionError("BSON mismatch");
    }
    public static void main(String[] args) throws Exception {
        KakaoSignatureResolver.sources.put("LocoHeader.kt",Arrays.asList(Header.class,NewHeader.class));
        KakaoSignatureResolver.sources.put("LocoBody.kt",Arrays.asList(Body.class,NewBody.class));
        packet(new Request()); packet(new Response()); packet(new NewRequest()); packet(new NewResponse());
        rejects(()->LocoPacketReader.verify(AmbiguousPacket.class));
        rejects(()->LocoPacketReader.verify(UnknownPacket.class));
        if (KakaoClassResolver.roomManager(LegacyManager.class)!=LegacyManager.companion.value) throw new AssertionError("legacy singleton");
        KakaoSignatureResolver.sources.put("ChatRoomListManagerProvider.kt",Arrays.asList(Provider.class));
        if (KakaoClassResolver.roomManager(CurrentManager.class)!=Provider.instance) throw new AssertionError("provider singleton");
        KakaoSignatureResolver.sources.put("ChatRoomListManagerProvider.kt",Arrays.asList(Provider.class,SecondProvider.class));
        rejects(()->KakaoClassResolver.roomManager(CurrentManager.class));
        KakaoSignatureResolver.sources.put("ChatRoomListManagerProvider.kt",Arrays.asList(NullProvider.class));
        rejects(()->KakaoClassResolver.roomManager(CurrentManager.class));
        System.out.println("10 packet/provider checks passed");
    }
}
