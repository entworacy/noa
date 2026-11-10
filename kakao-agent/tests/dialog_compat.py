"""Production overlay lifecycle against synthetic Android/JSON boundaries."""
import pathlib
import subprocess
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]

class DialogContract(unittest.TestCase):
    def test_background_overlay_lifecycle_and_failure_cleanup(self):
        sources = {
            "android/R.java": "package android; public class R { public static class style { public static final int Theme_Material_Light_Dialog_Alert=1; } }",
            "android/content/Context.java": "package android.content; public class Context {}",
            "android/app/ActivityThread.java": "package android.app; public class ActivityThread { public static final android.content.Context APP=new android.content.Context(); public static android.content.Context currentApplication(){return APP;} }",
            "android/provider/Settings.java": "package android.provider; public class Settings { public static boolean allowed; public static boolean canDrawOverlays(android.content.Context c){return allowed;} }",
            "android/os/Looper.java": "package android.os; public class Looper { public static final Looper MAIN=new Looper(); public static boolean onMain=true; public static Looper getMainLooper(){return MAIN;} public static Looper myLooper(){return onMain?MAIN:null;} }",
            "android/os/Handler.java": """package android.os;
                import java.util.*;
                public class Handler {
                    public static final List<Runnable> pending=new ArrayList<>();
                    public Handler(Looper l){if(l!=Looper.MAIN)throw new AssertionError();}
                    public boolean postDelayed(Runnable r,long ms){pending.add(r);return true;}
                    public void removeCallbacks(Runnable r){pending.remove(r);}
                    public static void fire(){ for(Runnable r:new ArrayList<Runnable>(pending))r.run(); }
                }""",
            "android/content/DialogInterface.java": """package android.content;
                public interface DialogInterface {
                    interface OnClickListener {void onClick(DialogInterface d,int which);}
                    interface OnCancelListener {void onCancel(DialogInterface d);}
                    interface OnDismissListener {void onDismiss(DialogInterface d);}
                }""",
            "android/app/AlertDialog.java": """package android.app;
                import android.content.*;
                import java.util.*;
                public class AlertDialog implements DialogInterface {
                    public static AlertDialog last;
                    public static boolean failShow,failDismiss;
                    public static int shownCount;
                    public final Window window=new Window();
                    public boolean showing,cancelable;
                    public String title,message;
                    public OnCancelListener cancel; public OnDismissListener dismiss;
                    public final List<OnClickListener> buttons=new ArrayList<>();
                    public static class Window {public int type; public void setType(int type){this.type=type;}}
                    public Window getWindow(){return window;}
                    public void setCanceledOnTouchOutside(boolean value){if(value!=cancelable)throw new AssertionError();}
                    public void setOnCancelListener(OnCancelListener value){cancel=value;}
                    public void setOnDismissListener(OnDismissListener value){dismiss=value;}
                    public void show(){if(window.type!=2038)throw new AssertionError("overlay type before show");if(failShow)throw new IllegalStateException("bad token");showing=true;last=this;shownCount++;}
                    public boolean isShowing(){return showing;}
                    public void dismiss(){if(failDismiss)throw new IllegalStateException("cannot dismiss");boolean was=showing;showing=false;if(was&&dismiss!=null)dismiss.onDismiss(this);}
                    public static class Builder {
                        final AlertDialog d=new AlertDialog();
                        public Builder(Context c,int theme){if(c!=ActivityThread.APP||theme!=1)throw new AssertionError("application context");}
                        public Builder setTitle(CharSequence s){d.title=s.toString();return this;}
                        public Builder setMessage(CharSequence s){d.message=s.toString();return this;}
                        public Builder setCancelable(boolean v){d.cancelable=v;return this;}
                        public Builder setPositiveButton(CharSequence s,OnClickListener l){d.buttons.add(l);return this;}
                        public Builder setNegativeButton(CharSequence s,OnClickListener l){d.buttons.add(l);return this;}
                        public Builder setNeutralButton(CharSequence s,OnClickListener l){d.buttons.add(l);return this;}
                        public AlertDialog create(){return d;}
                    }
                }""",
            "org/json/JSONObject.java": """package org.json;
                import java.util.*;
                public class JSONObject {
                    public static final Map<String,JSONObject> inputs=new HashMap<>();
                    public static Map<String,Object> last;
                    final Map<String,Object> data;
                    public JSONObject(String key){data=inputs.get(key).data;}
                    public JSONObject(Map<String,Object> value){data=new LinkedHashMap<>(value);last=data;}
                    public Object get(String key){if(!data.containsKey(key))throw new AssertionError(key);return data.get(key);}
                    public String toString(){return "fixture-json";}
                }""",
            "org/json/JSONArray.java": """package org.json;
                public class JSONArray {
                    final JSONObject[] values;
                    public JSONArray(JSONObject...v){values=v;}
                    public int length(){return values.length;}
                    public JSONObject getJSONObject(int i){return values[i];}
                }""",
            "Probe.java": """
                import java.util.*;
                import org.json.*;
                import android.app.AlertDialog;
                import android.os.*;
                import android.provider.Settings;
                import dev.noa.kakao.OverlayDialogs;
                public class Probe {
                    static JSONObject object(Object...pairs){Map<String,Object> m=new LinkedHashMap<>();for(int i=0;i<pairs.length;i+=2)m.put((String)pairs[i],pairs[i+1]);return new JSONObject(m);}
                    static Map<String,Object> call(String op,String id,boolean cancelable){
                        JSONObject content=object("title","Hello😀","message","line\\ntext", "cancelable",cancelable,"timeoutSeconds",60,
                            "buttons",new JSONArray(object("id","yes","label","Yes"),object("id","no","label","No"),object("id","other","label","Other")));
                        JSONObject.inputs.put("request",object("operation",op,"dialogId",id,"content",content));
                        if(!OverlayDialogs.execute("request").equals("fixture-json"))throw new AssertionError();
                        return JSONObject.last;
                    }
                    static void eq(Object a,Object b){if(!Objects.equals(a,b))throw new AssertionError(a+" != "+b);}
                    public static void main(String[] args) throws Exception {
                        eq(call("status","",true).get("overlayPermission"),false);
                        eq(call("show","denied",true).get("errorCode"),"permission-denied");
                        eq(AlertDialog.shownCount,0);
                        Settings.allowed=true;
                        eq(call("show","one",true).get("state"),"shown");
                        eq(AlertDialog.last.title,"Noa · Hello😀");eq(AlertDialog.last.window.type,2038);
                        eq(call("show","two",true).get("errorCode"),"dialog-active");
                        eq(call("show","one",true).get("state"),"shown");eq(AlertDialog.shownCount,1);
                        AlertDialog.last.buttons.get(1).onClick(AlertDialog.last,-2);
                        Map<String,Object> selected=call("get","one",true);
                        eq(selected.get("state"),"selected");eq(selected.get("buttonId"),"no");
                        eq(call("dismiss","one",true).get("state"),"selected");eq(Handler.pending.size(),0);
                        call("show","cancel",true);AlertDialog.last.cancel.onCancel(AlertDialog.last);
                        eq(call("get","cancel",true).get("state"),"cancelled");
                        call("show","dismiss",false);eq(AlertDialog.last.cancelable,false);
                        eq(call("dismiss","dismiss",false).get("state"),"dismissed");
                        call("show","timeout",true);Handler.fire();
                        eq(call("get","timeout",true).get("state"),"expired");eq(Handler.pending.size(),0);
                        eq(call("get","missing",true).get("errorCode"),"not-found");
                        AlertDialog.failShow=true;
                        eq(call("show","failure",true).get("errorCode"),"display-unavailable");eq(Handler.pending.size(),0);
                        AlertDialog.failShow=false;
                        call("show","retry",true);AlertDialog.failDismiss=true;
                        eq(call("dismiss","retry",true).get("errorCode"),"display-unavailable");
                        AlertDialog.failDismiss=false;eq(call("dismiss","retry",true).get("state"),"dismissed");
                        call("show","callback-failure",true);AlertDialog.failDismiss=true;
                        AlertDialog.last.buttons.get(0).onClick(AlertDialog.last,-1);
                        Map<String,Object> failed=call("get","callback-failure",true);
                        eq(failed.get("state"),"shown");if(failed.get("lastError")==null)throw new AssertionError();
                        Handler.fire();
                        AlertDialog.failDismiss=false;
                        eq(call("dismiss","callback-failure",true).get("lastError"),null);
                        for(int i=0;i<70;i++){String id="history"+i;call("show",id,true);call("dismiss",id,true);}
                        eq(call("status","",true).get("retainedResults"),64);
                        eq(call("get","one",true).get("errorCode"),"not-found");
                        Looper.onMain=false;
                        eq(call("status","",true).get("errorCode"),"display-unavailable");
                        System.out.println("Overlay permission, lifecycle, timeout, deduplication, bounded history and failure cleanup passed");
                    }
                }""",
        }
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            for name, content in sources.items():
                p = root / name
                p.parent.mkdir(parents=True, exist_ok=True)
                p.write_text(content)
            production = ROOT / "kakao-agent/java/dev/noa/kakao/OverlayDialogs.java"
            subprocess.run(["javac", "--release", "8", "-encoding", "UTF-8", "-d", tmp,
                            str(production), *map(str, root.rglob("*.java"))], check=True)
            subprocess.run(["java", "-cp", tmp, "Probe"], check=True)

if __name__ == "__main__":
    unittest.main()
