use jni::{Env, JValue, jni_sig, jni_str, objects::JObject};

pub enum Outbound<'a> {
    Words {
        referer: &'a str,
        room_id: i64,
        text: &'a str,
        thread_id: Option<i64>,
    },
    Files {
        room_id: i64,
        uris: &'a [String],
        mime: &'a str,
        title: Option<&'a str>,
    },
    Markdown {
        room_id: i64,
        text: &'a str,
    },
}

pub enum Delivery {
    Background,
    Foreground(String),
}

pub struct PreparedEnvelope<'local> {
    pub value: JObject<'local>,
    pub delivery: Delivery,
}

pub fn assemble<'local>(
    env: &mut Env<'local>,
    outbound: Outbound<'_>,
) -> jni::errors::Result<PreparedEnvelope<'local>> {
    match outbound {
        Outbound::Words {
            referer,
            room_id,
            text,
            thread_id,
        } => words(env, referer, room_id, text, thread_id),
        Outbound::Files {
            room_id,
            uris,
            mime,
            title,
        } => files(env, room_id, uris, mime, title),
        Outbound::Markdown { room_id, text } => markdown(env, room_id, text),
    }
}

fn markdown<'local>(
    env: &mut Env<'local>,
    room_id: i64,
    text: &str,
) -> jni::errors::Result<PreparedEnvelope<'local>> {
    let mut draft = IntentDraft::blank(env)?;
    draft.package("com.kakao.talk")?;
    draft.component("com.kakao.talk.activity.RecentExcludeIntentFilterActivity")?;
    draft.action("android.intent.action.SEND")?;
    draft.kind("text/plain")?;
    draft.text("android.intent.extra.TEXT", text)?;
    draft.text("EXTRA_CHAT_MESSAGE", text)?;
    draft.text("EXTRA_CHAT_ATTACHMENT", r#"{"markdown":true}"#)?;
    draft.integer("EXTRA_CHAT_MESSAGE_TYPE_VALUE", 1)?;
    draft.number("key_id", room_id)?;
    draft.integer("key_type", 1)?;
    draft.switch("key_from_direct_share", true)?;
    draft.flags(0x1000_0000)?;
    Ok(PreparedEnvelope {
        value: draft.release(),
        delivery: Delivery::Foreground("text/plain".to_string()),
    })
}

fn words<'local>(
    env: &mut Env<'local>,
    referer: &str,
    room_id: i64,
    text: &str,
    thread_id: Option<i64>,
) -> jni::errors::Result<PreparedEnvelope<'local>> {
    let mut draft = IntentDraft::blank(env)?;
    draft.component("com.kakao.talk.notification.NotificationActionService")?;
    draft.action("com.kakao.talk.notification.REPLY_MESSAGE")?;
    draft.text("noti_referer", referer)?;
    draft.number("chat_id", room_id)?;
    draft.switch("is_chat_thread_notification", thread_id.is_some())?;
    if let Some(thread_id) = thread_id {
        draft.number("thread_id", thread_id)?;
    }
    draft.remote_answer(text)?;
    Ok(PreparedEnvelope {
        value: draft.release(),
        delivery: Delivery::Background,
    })
}

fn files<'local>(
    env: &mut Env<'local>,
    room_id: i64,
    uris: &[String],
    mime: &str,
    title: Option<&str>,
) -> jni::errors::Result<PreparedEnvelope<'local>> {
    if uris.is_empty() {
        return Err(jni::errors::Error::NullPtr("KakaoTalk 공유 파일"));
    }
    let multiple = uris.len() > 1 || mime.starts_with("image/");
    let mut draft = IntentDraft::blank(env)?;
    draft.package("com.kakao.talk")?;
    draft.component("com.kakao.talk.activity.RecentExcludeIntentFilterActivity")?;
    draft.kind(if mime.starts_with("image/") {
        "image/*"
    } else {
        mime
    })?;
    draft.action(if multiple {
        "android.intent.action.SEND_MULTIPLE"
    } else {
        "android.intent.action.SEND"
    })?;
    if multiple {
        draft.uri_collection("android.intent.extra.STREAM", uris)?;
    } else {
        draft.uri("android.intent.extra.STREAM", &uris[0])?;
        if let Some(title) = title {
            draft.text("android.intent.extra.TITLE", title)?;
        }
    }
    draft.number("key_id", room_id)?;
    draft.integer("key_type", 1)?;
    draft.switch("key_from_direct_share", true)?;
    draft.flags(0x0000_0001 | 0x1000_0000 | 0x0400_0000)?;
    Ok(PreparedEnvelope {
        value: draft.release(),
        delivery: Delivery::Foreground(mime.to_string()),
    })
}

struct IntentDraft<'env, 'local> {
    env: &'env mut Env<'local>,
    value: JObject<'local>,
}

impl<'env, 'local> IntentDraft<'env, 'local> {
    fn blank(env: &'env mut Env<'local>) -> jni::errors::Result<Self> {
        let value = env.new_object(jni_str!("android/content/Intent"), jni_sig!("()V"), &[])?;
        Ok(Self { env, value })
    }

    fn release(self) -> JObject<'local> {
        self.value
    }

    fn component(&mut self, class_name: &str) -> jni::errors::Result<()> {
        let package = self.env.new_string("com.kakao.talk")?;
        let class_name = self.env.new_string(class_name)?;
        let component = self.env.new_object(
            jni_str!("android/content/ComponentName"),
            jni_sig!((pkg: java.lang.String, cls: java.lang.String) -> void),
            &[
                JValue::Object(&package.into()),
                JValue::Object(&class_name.into()),
            ],
        )?;
        self.env.call_method(
            &self.value,
            jni_str!("setComponent"),
            jni_sig!((component: android.content.ComponentName) -> android.content.Intent),
            &[JValue::Object(&component)],
        )?;
        Ok(())
    }

    fn action(&mut self, value: &str) -> jni::errors::Result<()> {
        let value = self.env.new_string(value)?;
        self.env.call_method(
            &self.value,
            jni_str!("setAction"),
            jni_sig!((action: java.lang.String) -> android.content.Intent),
            &[JValue::Object(&value.into())],
        )?;
        Ok(())
    }

    fn package(&mut self, value: &str) -> jni::errors::Result<()> {
        let value = self.env.new_string(value)?;
        self.env.call_method(
            &self.value,
            jni_str!("setPackage"),
            jni_sig!((package_name: java.lang.String) -> android.content.Intent),
            &[JValue::Object(&value.into())],
        )?;
        Ok(())
    }

    fn kind(&mut self, value: &str) -> jni::errors::Result<()> {
        let value = self.env.new_string(value)?;
        self.env.call_method(
            &self.value,
            jni_str!("setType"),
            jni_sig!((mime_type: java.lang.String) -> android.content.Intent),
            &[JValue::Object(&value.into())],
        )?;
        Ok(())
    }

    fn flags(&mut self, value: i32) -> jni::errors::Result<()> {
        self.env.call_method(
            &self.value,
            jni_str!("addFlags"),
            jni_sig!((flags: int) -> android.content.Intent),
            &[JValue::Int(value)],
        )?;
        Ok(())
    }

    fn text(&mut self, key: &str, value: &str) -> jni::errors::Result<()> {
        let key = self.env.new_string(key)?;
        let value = self.env.new_string(value)?;
        self.env.call_method(
            &self.value,
            jni_str!("putExtra"),
            jni_sig!((key: java.lang.String, value: java.lang.String) -> android.content.Intent),
            &[JValue::Object(&key.into()), JValue::Object(&value.into())],
        )?;
        Ok(())
    }

    fn number(&mut self, key: &str, value: i64) -> jni::errors::Result<()> {
        let key = self.env.new_string(key)?;
        self.env.call_method(
            &self.value,
            jni_str!("putExtra"),
            jni_sig!((key: java.lang.String, value: long) -> android.content.Intent),
            &[JValue::Object(&key.into()), JValue::Long(value)],
        )?;
        Ok(())
    }

    fn integer(&mut self, key: &str, value: i32) -> jni::errors::Result<()> {
        let key = self.env.new_string(key)?;
        self.env.call_method(
            &self.value,
            jni_str!("putExtra"),
            jni_sig!((key: java.lang.String, value: int) -> android.content.Intent),
            &[JValue::Object(&key.into()), JValue::Int(value)],
        )?;
        Ok(())
    }

    fn switch(&mut self, key: &str, value: bool) -> jni::errors::Result<()> {
        let key = self.env.new_string(key)?;
        self.env.call_method(
            &self.value,
            jni_str!("putExtra"),
            jni_sig!((key: java.lang.String, value: boolean) -> android.content.Intent),
            &[JValue::Object(&key.into()), JValue::Bool(value)],
        )?;
        Ok(())
    }

    fn uri(&mut self, key: &str, value: &str) -> jni::errors::Result<()> {
        let value = self.android_uri(value)?;
        let key = self.env.new_string(key)?;
        self.env.call_method(
            &self.value,
            jni_str!("putExtra"),
            jni_sig!((name: java.lang.String, value: android.os.Parcelable) -> android.content.Intent),
            &[JValue::Object(&key.into()), JValue::Object(&value)],
        )?;
        Ok(())
    }

    fn uri_collection(&mut self, key: &str, values: &[String]) -> jni::errors::Result<()> {
        let list = self
            .env
            .new_object(jni_str!("java/util/ArrayList"), jni_sig!("()V"), &[])?;
        for value in values {
            let uri = self.android_uri(value)?;
            self.env.call_method(
                &list,
                jni_str!("add"),
                jni_sig!((item: java.lang.Object) -> boolean),
                &[JValue::Object(&uri)],
            )?;
        }
        let key = self.env.new_string(key)?;
        self.env.call_method(
            &self.value,
            jni_str!("putParcelableArrayListExtra"),
            jni_sig!((name: java.lang.String, value: java.util.ArrayList) -> android.content.Intent),
            &[JValue::Object(&key.into()), JValue::Object(&list)],
        )?;
        Ok(())
    }

    fn android_uri(&mut self, value: &str) -> jni::errors::Result<JObject<'local>> {
        let value = self.env.new_string(value)?;
        self.env
            .call_static_method(
                jni_str!("android/net/Uri"),
                jni_str!("parse"),
                jni_sig!((uri_string: java.lang.String) -> android.net.Uri),
                &[JValue::Object(&value.into())],
            )?
            .l()
    }

    fn remote_answer(&mut self, text: &str) -> jni::errors::Result<()> {
        let results = self
            .env
            .new_object(jni_str!("android/os/Bundle"), jni_sig!("()V"), &[])?;
        let answer_key = self.env.new_string("reply_message")?;
        let answer = self.env.new_string(text)?;
        self.env.call_method(
            &results,
            jni_str!("putCharSequence"),
            jni_sig!((key: java.lang.String, value: java.lang.CharSequence) -> void),
            &[
                JValue::Object(&answer_key.into()),
                JValue::Object(&answer.into()),
            ],
        )?;

        let carrier =
            self.env
                .new_object(jni_str!("android/content/Intent"), jni_sig!("()V"), &[])?;
        let result_key = self.env.new_string("android.remoteinput.resultsData")?;
        self.env.call_method(
            &carrier,
            jni_str!("putExtra"),
            jni_sig!((key: java.lang.String, value: android.os.Bundle) -> android.content.Intent),
            &[JValue::Object(&result_key.into()), JValue::Object(&results)],
        )?;

        let label = self.env.new_string("android.remoteinput.results")?;
        let clip = self
            .env
            .call_static_method(
                jni_str!("android/content/ClipData"),
                jni_str!("newIntent"),
                jni_sig!((label: java.lang.CharSequence, intent: android.content.Intent) -> android.content.ClipData),
                &[JValue::Object(&label.into()), JValue::Object(&carrier)],
            )?
            .l()?;
        self.env.call_method(
            &self.value,
            jni_str!("setClipData"),
            jni_sig!((clip_data: android.content.ClipData) -> void),
            &[JValue::Object(&clip)],
        )?;
        Ok(())
    }
}
