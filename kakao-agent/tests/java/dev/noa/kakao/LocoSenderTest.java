package dev.noa.kakao;

import java.lang.reflect.Method;
import java.util.*;
import java.util.concurrent.atomic.AtomicInteger;

public final class LocoSenderTest {
    enum Command { PING, WRITE }
    static final class Header {
        final int a, d; final short b; final Command c;
        Header(int id, short status, Command method, int size) { a=id;b=status;c=method;d=size; }
    }
    static final class Body {
        final Map<String,Object> a;
        Body(Map<String,Object> value) { a=value; }
    }
    static class Packet {
        final Header header; final Body body;
        Packet(Header h, Body b) { header=h;body=b; }
    }
    static final class Request extends Packet {
        Request(Header h, Body b) { super(h,b); }
    }
    static final class Builder {
        final Command method; final int id; final short status;
        final Map<String,Object> values=new LinkedHashMap<>();
        Builder(Command method, int id, short status) { this.method=method;this.id=id;this.status=status; }
        Builder put(String key, Object value) { values.put(key,value);return this; }
        Request build() { return new Request(new Header(id,status,method,5+values.size()*10),new Body(values)); }
    }
    static final class Ids {
        static final Ids INSTANCE=new Ids();
        static final AtomicInteger counter=new AtomicInteger(1000);
        int next() { return counter.getAndIncrement(); }
    }
    static final class Client {
        final Hooker hook=new Hooker(1,false,1);
        final AtomicInteger active=new AtomicInteger();
        volatile boolean connected=true, fail=false;
        volatile Request last;
        Client() throws Exception { hook.backup=Client.class.getDeclaredMethod("write",Request.class); }
        boolean alive() { return connected; }
        void send(Request request) throws Throwable { hook.callback(new Object[]{this,request}); }
        void write(Request request) throws Exception {
            if(active.incrementAndGet()!=1) throw new AssertionError("concurrent socket writes");
            try {
                Thread.yield();
                if(fail) throw new IllegalStateException("socket write failed");
                last=request;
            } finally { active.decrementAndGet(); }
        }
    }
    interface Throwing { void run() throws Exception; }
    static void rejects(Throwing operation) throws Exception {
        try { operation.run(); } catch(ReflectiveOperationException | IllegalStateException | IllegalArgumentException expected) { return; }
        throw new AssertionError("invalid send accepted");
    }
    static void require(boolean value) { if(!value) throw new AssertionError(); }
    public static void main(String[] args) throws Throwable {
        KakaoSignatureResolver.sources.put("LocoHeader.kt",Arrays.asList(Header.class));
        KakaoSignatureResolver.sources.put("LocoBody.kt",Arrays.asList(Body.class));
        KakaoSignatureResolver.sources.put("LocoReq.kt",Arrays.asList(Request.class,Builder.class));
        KakaoSignatureResolver.sources.put("LocoJob.kt",Arrays.asList(Ids.class));
        Map<String,Object> body=new LinkedHashMap<>();
        body.put("id",Long.MAX_VALUE);body.put("items",Arrays.asList("한😀",null,true));body.put("binary",new byte[]{0,1,-1});
        rejects(()->LocoSender.send("PING",body));
        Client client=new Client();
        client.send(new Builder(Command.PING,999,(short)0).build());
        long[] sent=LocoSender.send("WRITE",body);
        require(sent[0]==1000 && sent[1]==35 && client.last.header.c==Command.WRITE);
        require(client.last.body.a.equals(body) && client.last.header.b==0);
        require(LocoSender.send("PING",Collections.emptyMap())[0]==1001);
        rejects(()->LocoSender.send("UNKNOWN",body));
        client.connected=false;rejects(()->LocoSender.send("PING",body));client.connected=true;
        KakaoLocoStateResolver.value=false;rejects(()->LocoSender.send("PING",body));KakaoLocoStateResolver.value=true;
        client.fail=true;rejects(()->LocoSender.send("PING",body));client.fail=false;
        Client replacement=new Client();replacement.send(new Builder(Command.PING,999,(short)0).build());
        LocoSender.send("WRITE",body);require(replacement.last.header.c==Command.WRITE);
        final Throwable[] errors=new Throwable[2];
        Thread normal=new Thread(()->{try {for(int i=0;i<100;i++) replacement.send(new Builder(Command.PING,99,(short)0).build());}catch(Throwable e){errors[0]=e;}});
        Thread api=new Thread(()->{try {for(int i=0;i<100;i++) LocoSender.send("WRITE",body);}catch(Throwable e){errors[1]=e;}});
        normal.start();api.start();normal.join();api.join();
        require(errors[0]==null && errors[1]==null);
        require(Bridge.captures.get()>=200);
        System.out.println("LOCO sender: construction, IDs, types, lifecycle, failures, capture and concurrent writes passed");
    }
}
// Platform/DEX entrypoints are fixtures; sender, packet reader and Hooker are real.
final class LocoSignatureResolver {
    static Object[] resolve() throws Exception { return new Object[]{LocoSenderTest.Client.class.getDeclaredMethod("send",LocoSenderTest.Request.class)}; }
}
final class KakaoLocoStateResolver {
    static boolean value=true;
    static boolean connected() { return value; }
}
final class Bridge {
    static final AtomicInteger captures=new AtomicInteger();
    static void capture(int kind,Object packet) { captures.incrementAndGet(); }
}
