import {
  CrmContext,
  EmptyState,
  ExtensionPointApiActions,
  Link,
  Text,
} from '@hubspot/ui-extensions';
import { hubspot } from '@hubspot/ui-extensions';

interface CrmExtensionProps {
  context: CrmContext;
  actions: ExtensionPointApiActions<'crm.record.tab'>;
}

hubspot.extend<'crm.record.tab'>(({ context, actions }: CrmExtensionProps) => (
  <DealQuoteCard context={context} actions={actions} />
));

const DealQuoteCard = ({ context }: CrmExtensionProps) => {
  const recordId = context.crm.objectId;
  const portalId = context.portal.id;
  const pasUrl = `https://pas.sagesure.io/quotes?hubspotPortalId=${encodeURIComponent(portalId)}&hubspotDealId=${encodeURIComponent(recordId)}`;

  return (
    <EmptyState
      title="SagePAS Deal Quote to Policy"
      layout="vertical"
      imageName="documents"
    >
      <Text>
        Create or reopen this deal&apos;s native quote, complete rating and
        underwriting, bind coverage, and issue the linked policy.
      </Text>
      <Link href={pasUrl}>Open deal quote-to-policy workflow in SagePAS</Link>
    </EmptyState>
  );
};
