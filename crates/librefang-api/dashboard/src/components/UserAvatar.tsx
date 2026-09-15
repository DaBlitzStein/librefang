import { type HTMLAttributes, memo } from "react";
import { Avatar, type AvatarSize } from "./ui/Avatar";
import { useUserAvatarUrl } from "../lib/queries/users";

interface UserAvatarProps extends HTMLAttributes<HTMLDivElement> {
  /**
   * The signed-in user's name.
   *
   * Doubles as the cache key and as the initials the fallback derives, and it
   * is deliberately not a path segment: the request goes to the literal
   * `/api/users/me/avatar` and the daemon resolves `me` from the credential.
   * See `currentUserAvatarPath` for why that matters — a name is
   * client-controlled, and a path built from one could only be admitted to the
   * authenticated-image allowlist by loosening it.
   */
  name: string;
  /** The user's emoji, shown when there is no image. */
  emoji?: string;
  size?: AvatarSize;
}

/**
 * The signed-in user's identity — image, then emoji, then initials (#8339).
 *
 * The mirror of `AgentAvatar`, and it exists for the same reason: the image is
 * fetched with the bearer credential and handed to an `img` as an object URL,
 * which is a hook, so it needs a component to live in.
 *
 * There is no "does this user have a picture" prop, unlike the agent one. An
 * agent list has twenty-two rows to spare a per-render 404 from, so the agent
 * hook is gated on `identity.avatar_url`; there is exactly one signed-in user,
 * and whether they have a picture is not known until the daemon answers — which
 * it does with a 404 the query reads as "no picture".
 */
export const UserAvatar = memo(function UserAvatar({
  name,
  emoji,
  size = "md",
  ...props
}: UserAvatarProps) {
  const src = useUserAvatarUrl(name);
  return <Avatar fallback={name} size={size} src={src} emoji={emoji} {...props} />;
});
